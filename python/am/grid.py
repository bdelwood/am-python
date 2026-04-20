from __future__ import annotations

import itertools
from concurrent.futures import ProcessPoolExecutor, as_completed
from typing import Callable

import numpy as np
import xarray as xr
from pathlib import Path

from am._am import Model


def _run_model(path: Path, args: list[str]) -> tuple[np.ndarray, dict[str, np.ndarray]]:
    """Worker: parse + compute one model, return frequency and output arrays."""
    m = Model(path, args)
    m.compute()
    return m.frequency.copy(), {k: v.copy() for k, v in m.raw_outputs.items()}


class ModelGrid:
    """Run an am model over a grid of parameter values.

    Parameters
    ----------
    path:
        Path to the ``.amc`` template file.
    params:
        xarray Dataset defining the parameter space.  Two forms are supported:

        * **Regular (Cartesian) grid** — coordinates on independent dimensions::

            xr.Dataset(coords={"elevation": [30, 45, 60], "pwv": [0.5, 1.0, 2.0]})

        * **Irregular grid** — variables sharing one dimension::

            xr.Dataset({"elevation": ("profile", elev), "pwv": ("profile", pwv)})

    args_fn:
        Callable that maps parameter values to an am args list.  Keyword
        argument names must match coordinate/variable names in *params*::

            args_fn=lambda elevation, pwv: [
                "0", "GHz", "350", "GHz", "0.5", "GHz",
                str(elevation), "deg", str(pwv),
            ]

    max_workers:
        Number of worker processes (default: ``os.cpu_count()``).
    """

    def __init__(
        self,
        path: Path,
        params: xr.Dataset,
        args_fn: Callable[..., list[str]],
        max_workers: int | None = None,
    ):
        self._path = path
        self._params = params
        self._args_fn = args_fn
        self._max_workers = max_workers

    def _iter_points(self) -> list[tuple[dict, dict]]:
        dim_names = list(self._params.sizes)
        points = []
        for indices in itertools.product(
            *[range(n) for n in self._params.sizes.values()]
        ):
            isel = dict(zip(dim_names, indices))
            pt = self._params.isel(isel)
            # Irregular grids: physical values are data_vars (e.g. elevation, pwv).
            # Regular Cartesian grids: data_vars is empty; values are dimension coords.
            sources = pt.data_vars if pt.data_vars else pt.coords
            kwargs = {name: float(pt[name]) for name in sources}
            points.append((isel, kwargs))
        return points

    def compute(self) -> xr.Dataset:
        """Run all models and return an xarray Dataset.

        The output Dataset has the same dimensions and coordinates as *params*,
        plus a ``frequency`` dimension.  Each requested am output (e.g.
        ``tb_rj``, ``transmittance``) becomes a data variable.
        """
        points = self._iter_points()
        results: list = [None] * len(points)

        with ProcessPoolExecutor(max_workers=self._max_workers) as ex:
            futures = {
                ex.submit(_run_model, self._path, self._args_fn(**kw)): i
                for i, (_, kw) in enumerate(points)
            }
            for fut in as_completed(futures):
                results[futures[fut]] = fut.result()

        freq, first_outputs = results[0]
        available = list(first_outputs.keys())

        dim_names = list(self._params.sizes)
        shape = tuple(self._params.sizes.values())

        out_arrays = {name: np.empty(shape + (len(freq),)) for name in available}
        for (isel, _), (_, outputs) in zip(points, results):
            idx = tuple(isel[d] for d in dim_names)
            for name in available:
                out_arrays[name][idx] = outputs[name]

        # Carry params coords and data_vars into the output.
        # For irregular grids this promotes physical variables (elevation, pwv)
        # to non-dimension coordinates so they're accessible on the output Dataset.
        coords: dict = dict(self._params.coords)
        coords.update({name: self._params[name] for name in self._params.data_vars})
        coords["frequency"] = freq

        return xr.Dataset(
            {
                name: (dim_names + ["frequency"], arr)
                for name, arr in out_arrays.items()
            },
            coords=coords,
        )
