"""Smoke tests for the pourpoint Python bindings."""


def test_import():
    """Module can be imported."""
    import pourpoint

    assert hasattr(pourpoint, "Engine")


def test_exceptions_exist():
    """Custom exception classes are importable."""
    from pourpoint import DatasetError, ResolutionError, PourpointError

    assert issubclass(DatasetError, PourpointError)
    assert issubclass(ResolutionError, PourpointError)


def test_engine_bad_path():
    """Engine raises DatasetError for a nonexistent path."""
    import pytest

    import pourpoint

    with pytest.raises(pourpoint.DatasetError):
        pourpoint.Engine("/nonexistent/path/to/dataset")
