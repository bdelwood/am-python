from __future__ import annotations

import numpy as np
import xarray as xr

from am._am import Model as _Model


class Model:
    """An am atmospheric model loaded from an ``.amc`` configuration file.

    Thin wrapper around the Rust ``_Model`` that returns xarray Datasets
    for :attr:`outputs` and :meth:`jacobian`.

    Parameters
    ----------
    path:
        Path to the ``.amc`` file.
    args:
        Positional substitution values for ``%1``, ``%2``, … placeholders
        in the config (frequency grid, zenith angle, PWV scale, etc.).
    """

    def __init__(self, path, args):
        self._inner = _Model(path, args)

    def compute(self):
        """Run the radiative transfer computation."""
        self._inner.compute()

    @property
    def frequency(self) -> np.ndarray:
        """Frequency grid in GHz."""
        return self._inner.frequency

    @property
    def outputs(self) -> xr.Dataset:
        """Computed output spectra as an xarray Dataset.

        Dimension is ``frequency`` (GHz).  Only outputs listed in the
        ``output`` directive of the ``.amc`` file are present.
        Empty before :meth:`compute` is called.
        """
        raw = self._inner.raw_outputs
        freq = self._inner.frequency
        return xr.Dataset(
            {name: ("frequency", arr) for name, arr in raw.items()},
            coords={"frequency": freq},
        )

    @property
    def variables(self) -> list[str]:
        """Names of fit/differentiation variables defined in the config."""
        return self._inner.variables

    @property
    def n_variables(self) -> int:
        """Number of fit/differentiation variables."""
        return self._inner.n_variables

    def jacobian(self) -> xr.Dataset:
        """Compute Jacobians of all outputs w.r.t. fit variables.

        Returns an xarray Dataset with dimensions ``(variable, frequency)``.
        The ``.amc`` config must define fit variables (parameters with scales).

        Raises
        ------
        ConfigError
            If no fit variables are defined in the ``.amc`` config.
        """
        raw = self._inner.raw_jacobian()
        freq = self._inner.frequency
        var_names = self._inner.variables
        return xr.Dataset(
            {name: (("variable", "frequency"), arr) for name, arr in raw.items()},
            coords={"variable": var_names, "frequency": freq},
        )

    def summary(self) -> str:
        """Full resolved model configuration summary."""
        return self._inner.summary()

    def __str__(self) -> str:
        return self.summary()
