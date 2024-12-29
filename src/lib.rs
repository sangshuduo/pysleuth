use pyo3::prelude::*;
use pyo3::wrap_pyfunction;

/// A simple function exposed to Python under the name `main`.
#[pyfunction]
fn main_py() -> PyResult<()> {
    // Your Rust logic here, for example:
    println!("Hello from Rust!");
    run_main_logic().unwrap();
    // Return an empty Ok(()) to satisfy the PyResult<()> type.
    Ok(())
}

#[pyfunction]
fn version_py() -> PyResult<String> {
    Ok(env!("CARGO_PKG_VERSION").to_string())
}

#[pymodule]
fn pysleuth(_py: Python, m: &PyModule) -> PyResult<()> {
    // Register the version_py function with the Python module.
    m.add_function(wrap_pyfunction!(version_py, m)?)?;
    // Add the `main` function to the module.
    // so that Python code can call `python_code_analyzer.main_py()`.
    m.add_function(wrap_pyfunction!(main_py, m)?)?;

    // Return Ok(()) to satisfy the PyResult<()> return type
    Ok(())
}

use anyhow::Result;
use clap::{Arg, Command};
use colored::*;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use tree_sitter::{Node, Parser};

fn run_main_logic() -> Result<()> {
    // Fetch metadata from Cargo.toml using env! macros
    let name = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");
    let author = env!("CARGO_PKG_AUTHORS");
    let description = env!("CARGO_PKG_DESCRIPTION");

    let matches = Command::new(name)
        .version(version)
        .author(author)
        .about(description)
        .arg(
            Arg::new("files")
                .help("Python files to analyze")
                .required(true)
                .num_args(1..)
                .value_hint(clap::ValueHint::FilePath),
        )
        .get_matches();

    // Get the list of files to analyze
    let files: Vec<&str> = matches
        .get_many::<String>("files")
        .unwrap()
        .map(|s| s.as_str())
        .collect();

    // Process each file
    for filename in files {
        match fs::read_to_string(filename) {
            Ok(source_code) => {
                if let Err(e) = analyze_file(filename, &source_code) {
                    eprintln!("Error analyzing file '{}': {}", filename, e);
                }
            }
            Err(e) => {
                eprintln!("Error reading file '{}': {}", filename, e);
            }
        }
    }

    Ok(())
}

fn analyze_file(filename: &str, source_code: &str) -> Result<()> {
    // Initialize the parser with the Python grammar
    let language = tree_sitter_python::LANGUAGE;
    let mut parser = Parser::new();
    parser
        .set_language(&language.into())
        .expect("Error loading Python grammar");

    // Parse the source code
    let tree = parser.parse(source_code, None).unwrap();

    // Collect all functions
    let mut functions = HashMap::new();
    collect_functions(tree.root_node(), &mut functions, source_code);

    // Include the module-level code as a function
    functions.insert(
        "<module>".to_string(),
        FunctionInfo {
            node: tree.root_node(),
            may_raise: HashSet::new(),
            reported_in_function: Cell::new(false),
        },
    );

    // Determine exceptions each function may raise
    determine_exceptions(&mut functions, source_code);

    // Analyze each function
    let mut reported_calls = HashSet::new();
    for func_name in functions.keys() {
        analyze_function(
            func_name,
            functions[func_name].node,
            &functions,
            source_code,
            filename,
            &mut reported_calls,
        );
    }

    Ok(())
}

struct FunctionInfo<'a> {
    node: Node<'a>,
    may_raise: HashSet<String>,
    reported_in_function: Cell<bool>,
}

struct FunctionCall<'a> {
    name: String,
    node: Node<'a>,
}

fn collect_functions<'a>(
    node: Node<'a>,
    functions: &mut HashMap<String, FunctionInfo<'a>>,
    source_code: &str,
) {
    let mut cursor = node.walk();
    if node.kind() == "function_definition" {
        let name_node = node.child_by_field_name("name").unwrap();
        let name = name_node
            .utf8_text(source_code.as_bytes())
            .unwrap()
            .to_string();
        functions.insert(
            name.clone(),
            FunctionInfo {
                node,
                may_raise: HashSet::new(),
                reported_in_function: Cell::new(false),
            },
        );
    }

    // Traverse child nodes
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            collect_functions(child, functions, source_code);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn collect_function_calls<'a>(
    node: Node<'a>,
    calls: &mut Vec<FunctionCall<'a>>,
    source_code: &str,
) {
    let mut cursor = node.walk();
    if node.kind() == "call" {
        if let Some(function_node) = node.child_by_field_name("function") {
            let name = function_node
                .utf8_text(source_code.as_bytes())
                .unwrap()
                .to_string();
            calls.push(FunctionCall { name, node });
        }
    }

    // Traverse child nodes
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            collect_function_calls(child, calls, source_code);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn determine_exceptions(functions: &mut HashMap<String, FunctionInfo<'_>>, source_code: &str) {
    let function_names: Vec<String> = functions.keys().cloned().collect();
    let mut changed = true;
    while changed {
        changed = false;
        for func_name in &function_names {
            let mut new_exceptions = HashSet::new();

            // Use an immutable reference to `func_info`
            let func_info = &functions[func_name];

            // Collect exceptions from unguarded dict accesses in the function
            let mut unguarded_accesses = Vec::new();
            find_unguarded_dict_accesses(func_info.node, &mut unguarded_accesses, source_code);
            for access_node in unguarded_accesses {
                if !is_within_keyerror_try_except(access_node, source_code) {
                    new_exceptions.insert("KeyError".to_string());
                }
            }

            // Collect exceptions from called functions
            let mut calls = Vec::new();
            collect_function_calls(func_info.node, &mut calls, source_code);
            for call in calls {
                if let Some(called_func) = functions.get(&call.name) {
                    let exceptions = &called_func.may_raise;
                    if !exceptions.is_empty()
                        && !is_within_keyerror_try_except(call.node, source_code)
                    {
                        new_exceptions.extend(exceptions.clone());
                    }
                }
            }

            // Now, limit the mutable borrow of `func_info` to this block
            {
                let func_info_mut = functions.get_mut(func_name).unwrap();

                // Check if the exceptions set has changed
                if !new_exceptions.is_subset(&func_info_mut.may_raise) {
                    func_info_mut.may_raise.extend(new_exceptions);
                    changed = true;
                }
            } // Mutable borrow ends here
        }
    }
}

fn analyze_function<'a>(
    function_name: &str,
    _function_node: Node<'a>,
    functions: &HashMap<String, FunctionInfo<'a>>,
    source_code: &str,
    filename: &str,
    reported_calls: &mut HashSet<(usize, String)>,
) {
    let func_info = functions.get(function_name).unwrap();

    // Split source code into lines
    let source_lines: Vec<&str> = source_code.lines().collect();

    // Check for unguarded dict accesses within the function
    let mut unguarded_accesses = Vec::new();
    find_unguarded_dict_accesses(func_info.node, &mut unguarded_accesses, source_code);

    if !unguarded_accesses.is_empty() {
        // Report warning for unguarded dict access
        for access_node in unguarded_accesses {
            if !is_within_keyerror_try_except(access_node, source_code) {
                let start_position = access_node.start_position();
                let end_position = access_node.end_position();
                let line_number = start_position.row + 1;
                let column_start = start_position.column;
                let column_end = end_position.column;

                let line = source_lines.get(start_position.row).unwrap_or(&"");
                if function_name != "<module>" {
                    println!(
                        "{}:{}:{}: {} Possible KeyError in function '{}'",
                        filename,
                        line_number,
                        column_start + 1,
                        "Warning:".yellow().bold(),
                        function_name
                    );

                    // Print the code line
                    println!("{}|", line_number.to_string().blue());
                    println!(
                        "{}| {}",
                        " ".repeat(line_number.to_string().len()).blue(),
                        line
                    );

                    // Print the indicator line
                    let indicator = format!(
                        "{}{}",
                        " ".repeat(column_start),
                        "^".repeat(std::cmp::max(1, column_end - column_start))
                    );
                    println!(
                        "{}| {}",
                        " ".repeat(line_number.to_string().len()).blue(),
                        indicator.bright_red()
                    );

                    // Add a blank line for better readability
                    println!();
                }
            }
        }

        // Mark the function as having reported unhandled exceptions
        func_info.reported_in_function.set(true);
    }

    // Check for unhandled exceptions at call sites
    let mut calls = Vec::new();
    collect_function_calls(func_info.node, &mut calls, source_code);

    for call in calls {
        if let Some(called_func) = functions.get(&call.name) {
            let exceptions = &called_func.may_raise;
            if !exceptions.is_empty() && !is_within_keyerror_try_except(call.node, source_code) {
                let start_position = call.node.start_position();
                let end_position = call.node.end_position();
                let line_number = call.node.start_position().row + 1;
                let column_start = start_position.column;
                let column_end = end_position.column;
                let key = (line_number, call.name.clone());

                // Only report if not already reported in the called function
                if !reported_calls.contains(&key) && !called_func.reported_in_function.get() {
                    reported_calls.insert(key);

                    let line = source_lines.get(start_position.row).unwrap_or(&"");
                    println!(
                        "{}:{}:{}: {} Possible {} not handled when calling '{}' in function '{}'",
                        filename,
                        line_number,
                        column_start + 1,
                        "Warning:".yellow().bold(),
                        exceptions
                            .iter()
                            .cloned()
                            .collect::<Vec<String>>()
                            .join(", "),
                        call.name,
                        function_name
                    );

                    // Print the code line
                    println!("{}|", line_number.to_string().blue());
                    println!(
                        "{}| {}",
                        " ".repeat(line_number.to_string().len()).blue(),
                        line
                    );

                    // Print the indicator line
                    let indicator = format!(
                        "{}{}",
                        " ".repeat(column_start),
                        "^".repeat(std::cmp::max(1, column_end - column_start))
                    );
                    println!(
                        "{}| {}",
                        " ".repeat(line_number.to_string().len()).blue(),
                        indicator.bright_red()
                    );

                    // Add a blank line for better readability
                    println!();
                }
            }
        }
    }
}

fn find_unguarded_dict_accesses<'a>(
    node: Node<'a>,
    accesses: &mut Vec<Node<'a>>,
    source_code: &str,
) {
    let mut cursor = node.walk();
    if node.kind() == "subscript" {
        // Check if it's inside a try/except KeyError block
        if !is_within_keyerror_try_except(node, source_code) {
            accesses.push(node);
        }
    }

    // Traverse child nodes
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            find_unguarded_dict_accesses(child, accesses, source_code);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn is_within_keyerror_try_except(node: Node, source_code: &str) -> bool {
    let mut current_node = node;
    loop {
        if current_node.kind() == "try_statement" {
            // Check except clauses
            let mut cursor = current_node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "except_clause" {
                        if let Some(exception_type) = child.child_by_field_name("type") {
                            let exception_text =
                                exception_type.utf8_text(source_code.as_bytes()).unwrap();
                            if exception_text == "KeyError" || exception_text == "Exception" {
                                return true;
                            }
                        } else {
                            // Bare except
                            return true;
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        if let Some(parent) = current_node.parent() {
            current_node = parent;
        } else {
            break;
        }
    }
    false
}
