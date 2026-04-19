import logging

import am
import pytest


def test_file_not_found():
    with pytest.raises(am.ConfigError, match="cannot open file"):
        am.Model("nonexistent.amc", [])


def test_missing_args(amc):
    """Missing %N args should show the file:line context and embedded help text."""
    with pytest.raises(am.ConfigError, match=r"(?s)%1 has no matching.*Usage:"):
        am.Model(amc, [])


def test_base_class_catches():
    with pytest.raises(am.AmError):
        am.Model("nonexistent.amc", [])


def test_exception_hierarchy():
    assert issubclass(am.ConfigError, am.AmError)
    assert issubclass(am.ComputeError, am.AmError)


def test_warnings_emitted(amc, args, caplog):
    with caplog.at_level(logging.WARNING, logger="am.models"):
        m = am.Model(amc, args)
        m.compute()

    assert "narrower than the frequency" in caplog.text


def test_no_stale_error_in_warnings(amc, args, caplog):
    """After a failed Model(), warnings from a successful run should not
    contain stale '! Error:' entries."""
    with pytest.raises(am.ConfigError):
        am.Model("nonexistent.amc", [])

    with caplog.at_level(logging.WARNING, logger="am.models"):
        m = am.Model(amc, args)
        m.compute()

    assert "! Error:" not in caplog.text
