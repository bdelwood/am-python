import pytest

import am


JACOBIAN_AMC = "assets/MaunaKea_Jacobian.amc"
JACOBIAN_ARGS = [220, "GHz", 230, "GHz", 5, "GHz", 0, "deg", 277, "K", 1.0]


def test_no_variables_raises(model):
    with pytest.raises(am.ConfigError, match="No fit variables"):
        model.jacobian()


def test_jacobian_shape_and_coords():
    m = am.Model(JACOBIAN_AMC, JACOBIAN_ARGS)
    m.compute()
    jac = m.jacobian()

    assert set(jac.dims) == {"variable", "frequency"}
    assert jac.sizes["variable"] == 1
    assert jac.sizes["frequency"] == 3
    assert "Nscale troposphere h2o" in jac.coords["variable"].values
    assert "tb_rj" in jac.data_vars


def test_jacobian_works_without_prior_compute():
    m = am.Model(JACOBIAN_AMC, JACOBIAN_ARGS)
    jac = m.jacobian()
    assert "tb_rj" in jac.data_vars
    assert "tb_rj" in m.outputs.data_vars
