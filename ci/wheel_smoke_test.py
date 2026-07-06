"""Wheel smoke test for the Windows cibuildwheel leg.

Byte-for-byte the same assertions as the inline `python -c` in
CIBW_TEST_COMMAND (build-wheels.yaml); kept as a file because cibuildwheel
runs Windows commands under cmd.exe, which cannot execute a multi-line
quoted -c program. Keep the two in sync.
"""

import pourpoint
from pathlib import Path

pkg = Path(pourpoint.__file__).parent
assert (pkg / '_data' / 'gdal' / 'gdalvrt.xsd').is_file(), 'missing bundled gdal_data'
assert (pkg / '_data' / 'proj' / 'proj.db').is_file(), 'missing bundled proj.db'
assert pourpoint.__version__, 'missing __version__'
try:
    pourpoint.Engine('/nonexistent/path/to/dataset')
except pourpoint.DatasetError:
    pass
else:
    raise AssertionError('Engine should reject missing datasets')
from pourpoint import _pourpoint
_pourpoint._self_test_proj()
print('wheel self-test passed; version=' + pourpoint.__version__)
