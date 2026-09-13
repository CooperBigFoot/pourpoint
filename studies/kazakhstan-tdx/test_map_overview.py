"""Map-only geometry and report contract checks; no delivery map is rendered."""
import importlib.util
from pathlib import Path
import unittest

from shapely.geometry import Point, Polygon, box

spec = importlib.util.spec_from_file_location("map_overview", Path(__file__).with_name("map_overview.py"))
map_overview = importlib.util.module_from_spec(spec)
spec.loader.exec_module(map_overview)


class OverviewGeometryTests(unittest.TestCase):
    def test_extent_retains_cross_border_watershed_and_western_exclusion(self):
        kaz = box(100, 100, 200, 200)
        basin = box(50, 80, 230, 260)
        excluded = Point(-80, 120)
        west, south, east, north = map_overview.full_extent([kaz, basin, excluded])
        self.assertLess(west, -80)
        self.assertLess(south, 80)
        self.assertGreater(east, 230)
        self.assertGreater(north, 260)

    def test_suspect_station_is_not_relocated(self):
        point = map_overview.usable_coordinate({"longitude": "52.4317", "latitude": "52.4317"})
        self.assertEqual((point.x, point.y), (52.4317, 52.4317))

    def test_unusable_coordinates_have_no_marker(self):
        for longitude, latitude in [("nan", "45"), ("70", "inf"), ("", "45"), ("181", "45")]:
            with self.subTest(longitude=longitude, latitude=latitude):
                self.assertIsNone(map_overview.usable_coordinate({"longitude": longitude, "latitude": latitude}))

    def test_polygon_holes_survive_display_path(self):
        import matplotlib.pyplot as plt
        polygon = Polygon([(0, 0), (4, 0), (4, 4), (0, 4)],
                          holes=[[(1, 1), (3, 1), (3, 3), (1, 3)]])
        fig, axes = plt.subplots()
        map_overview.draw_polygons(axes, polygon)
        path = axes.patches[0].get_path()
        self.assertEqual(list(path.codes).count(map_overview.PlotPath.MOVETO), 2)
        rings = path.to_polygons()
        signed_areas = [sum(a[0]*b[1]-b[0]*a[1] for a, b in zip(ring, ring[1:])) / 2 for ring in rings]
        self.assertGreater(signed_areas[0], 0)
        self.assertLess(signed_areas[1], 0)
        plt.close(fig)


if __name__ == "__main__":
    unittest.main()
