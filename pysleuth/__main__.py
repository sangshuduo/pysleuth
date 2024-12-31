import sys
from .pysleuth import version_py, run_command

def main():
    if "--version" in sys.argv:
        print(version_py())
    else:
        # Example: pass everything except the script name to Rust:
        args = " ".join(sys.argv[1:])
        try:
            run_command(args)
        except Exception as e:
            sys.exit(1)
        else:
            sys.exit(0)

if __name__ == "__main__":
    main()
