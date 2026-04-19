import am
import pytest

AMC = "assets/SPole_JJA_75.amc"
ARGS = [0, "GHz", 350, "GHz", 0.5, "GHz", 35, "deg", 1.0]


@pytest.fixture
def amc():
    return AMC


@pytest.fixture
def args():
    return list(ARGS)


@pytest.fixture
def model():
    m = am.Model(AMC, ARGS)
    m.compute()
    return m
