# am-python

Python bindings for Scott Paine's [am atmospheric model](https://lweb.cfa.harvard.edu/~spaine/am/), via Rust (PyO3).

## Install

Requires the [am source code](https://doi.org/10.5281/zenodo.8161261) (v14.0) and a C compiler.

```bash
# Download and extract am source
curl -fsSL "https://zenodo.org/records/13748403/files/am-14.0.tgz?download=1" | tar -xz
export AM_SRC_DIR=$PWD/am-14.0/src

# Install
uv pip install .
```

## Usage

```python
import am

m = am.Model("SPole_JJA_75.amc", ["0", "GHz", "350", "GHz", "0.01", "GHz", "35", "deg", "1.0"])
m.compute()

m.frequency       # numpy array, GHz
m.transmittance   # numpy array
m.opacity         # numpy array, nepers
m.tb_planck       # numpy array, K
```
