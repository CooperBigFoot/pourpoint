# GRIT-derived regional watershed regression fixture

`tiny-components.wkb` is GRIT-derived data, NOT covered by pourpoint's MIT
software license. It is distributed under [CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/).
Use permitted by this license is for NonCommercial purposes only. The MIT
license does not grant commercial rights to this fixture.

Source creators: Michel Wortmann, Louise Slater, Laurence Hawker, Yinxue Liu,
and Jeffrey Neal. Research-group contributor: EvoFlood Team.

Sources (version 1.0):
- [Global River Topology (GRIT) vector datasets](https://doi.org/10.5281/zenodo.17435232)
- [Global River Topology (GRIT) raster datasets](https://doi.org/10.5281/zenodo.15715535)
- [Global River Topology (GRIT): A Bifurcating River Hydrography](https://doi.org/10.1029/2024WR038308), Water Resources Research 61(5).

GRIT v1.0 was compiled into the hosted GRIT 2.0.0 HFX dataset (format 0.3.0;
adapter grit-global-2.1.0) at
https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/.
Pourpoint produced the captured Basel watershed on 2026-09-09 at commit
f268c70b6f94bb2a404360cb395686a7dc865f89, using unchanged defaults for
`Engine(uri).delineate(47.5596, 7.5890)`.

Derivation: only zero-based components 1 and 8 were selected from the captured
nine-part MultiPolygon. Their complete polygon WKB byte slices were copied
without rounding, coordinate changes, reorientation or repair. A new
little-endian MultiPolygon header declares two parts. This is a reduced derived
output, not an unmodified upstream dataset. Each polygon is 77 bytes; the
fixture is 163 bytes. No complete operational watershed is distributed.

The data is supplied as-is and as-available, without warranties. The upstream
warranty disclaimer and limitation of liability apply; see
[Section 5](https://creativecommons.org/licenses/by-nc/4.0/legalcode.en).
No source-creator endorsement is implied. Upstream Tech provides hosting
infrastructure only.

License investigation: the vector and raster Zenodo record APIs and the vector
record's `LICENSE.md` both identify CC BY-NC-4.0 (checked 2026-09-10).
This notice does not determine whether a particular downstream commercial CI
use qualifies as NonCommercial.

## Integrity

Captured full WKB SHA256: `245baf699c504361240bbf670c7e5a774a8fa6e1e44f5678a63577eb66a63809`.

Fixture SHA256: `0c22fcf9569b15143f96ba0b207293e946cc53e48262d4861656c1c642376fe1`.
