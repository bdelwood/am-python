# am-python

<!-- readme-include-start -->

[![CI status][ci-img]][ci-url]
[![Documentation][doc-img]][doc-url]
[![PyPI version][pypi-img]][pypi-url]
[![Wheels][wheels-img]][wheels-url]
[![License][license-img]][license-url]

[ci-img]: https://img.shields.io/github/actions/workflow/status/bdelwood/am-python/ci.yaml?branch=master&style=flat-square&label=CI
[ci-url]: https://github.com/bdelwood/am-python/actions/workflows/ci.yaml
[doc-img]: https://img.shields.io/badge/docs-am--python-4d76ae?style=flat-square
[doc-url]: https://bdelwood.github.io/am-python/
[pypi-img]: https://img.shields.io/pypi/v/am-python?style=flat-square
[pypi-url]: https://pypi.org/project/am-python/
[wheels-img]: https://img.shields.io/github/actions/workflow/status/bdelwood/am-python/release.yaml?branch=master&style=flat-square&label=Wheels
[wheels-url]: https://github.com/bdelwood/am-python/actions/workflows/release.yaml
[license-img]: https://img.shields.io/badge/license-MIT-yellow?style=flat-square
[license-url]: https://github.com/bdelwood/am-python/blob/master/LICENSE

Python bindings for Scott Paine's [am atmospheric model](https://lweb.cfa.harvard.edu/~spaine/am/), via Rust (PyO3).

## Install

Pre-built wheels are available on PyPI for Linux:

```bash
uv pip install am-python
```

### From source

Requires the [am source code](https://doi.org/10.5281/zenodo.8161261) (tested on v14.0), a C compiler, and a Rust toolchain.

```bash
curl -fsSL "https://zenodo.org/records/13748403/files/am-14.0.tgz?download=1" | tar -xz
export AM_SRC_DIR=$PWD/am-14.0/src
uv pip install .
```

## Usage

```python
import am

m = am.Model("SPole_JJA_75.amc", [0, "GHz", 350, "GHz", 0.01, "GHz", 35, "deg", 1.0])
m.compute()

m.frequency       # numpy array, GHz
m.transmittance   # numpy array
m.opacity         # numpy array, nepers
m.tb_planck       # numpy array, K
```

## Development

This project uses `just` to orchestrate common tasks and `pre-commit` for local checks.

```bash
# install deps
just sync

# run tests
just test
just test py
just test rs

# run formatting/lint checks
just fmt
just fmt-check
just lint
just typecheck

# build docs
just docs
just docs py
just docs rs

# run pre-commit hooks on all files
just precommit
just prepush
```
