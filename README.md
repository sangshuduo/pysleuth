# PySleuth

Hunging down your Python code errors with PySleuth.

## Installation

```bash
pip install pysleuth
```

## Build and Publish

```bash
$ maturin build --release

# macOS
$ twine upload target/wheels/pysleuth-<VERSION>-cp310-cp310-macosx_11_0_arm64.whl

# Linux
$ twine upload target/wheels/pysleuth-<VERSION>-cp310-cp310-manylinux_2_34_x86_64.whl
```
