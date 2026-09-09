# Geometry regression inputs

`point-tangent-hole.wkb` is a synthetic polygon. Its shell is the square
(0, 0), (4, 0), (4, 4), (0, 4). Its diamond hole is (0, 2),
(1, 1), (2, 2), (1, 3). Both rings are closed. The hole touches the
shell at one point. GEOS 3.13.1 reports it valid. No operational data is
included. The default hole policy must fill the diamond rather than embed
its edges in a self-touching exterior ring.
