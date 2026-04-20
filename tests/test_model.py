import numpy as np


def test_sequential_runs(amc, args):
    """Three sequential model runs should all produce identical, valid results."""
    import am

    for i in range(3):
        m = am.Model(amc, args)
        m.compute()
        assert m.frequency.shape == (701,), f"run {i}: wrong grid size"
        assert "transmittance" in m.outputs, f"run {i}: no transmittance"
        np.testing.assert_allclose(m.outputs["transmittance"][300], 0.9707, atol=1e-3)
        del m


def test_summary(model):
    summary = model.summary()
    assert "am version" in summary
    assert "f 0 GHz" in summary
