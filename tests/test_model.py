import am
import numpy as np

AMC = "assets/SPole_JJA_75.amc"
ARGS = ["0", "GHz", "350", "GHz", "0.5", "GHz", "35", "deg", "1.0"]


def test_sequential_runs():
    """Three sequential model runs should all produce identical, valid results."""
    for i in range(3):
        m = am.Model(AMC, ARGS)
        m.compute()
        assert m.frequency.shape == (701,), f"run {i}: wrong grid size"
        assert m.transmittance is not None, f"run {i}: no transmittance"
        np.testing.assert_allclose(m.transmittance[300], 0.9707, atol=1e-3)
        del m
