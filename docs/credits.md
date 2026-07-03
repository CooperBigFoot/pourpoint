# Credits & Citation

## The algorithm is Matthew Heberger's

The delineation method at the heart of `shed` — the hybrid of raster and vector
techniques described in [How it works](how-it-works.md) — is the work of
**Matthew Heberger** ([ORCID 0000-0001-9122-0030](https://orcid.org/0000-0001-9122-0030)).
He designed the method, validated it against reference watersheds, published it, and
released a reference implementation as the open-source, **MIT**-licensed
[delineator](https://github.com/mheberger/delineator) tool, alongside the
point-and-click [Global Watersheds](https://mghydro.com/watersheds) web application.

`shed` is an independent Rust reimplementation of that method. It does not extend or
improve the algorithm — the intellectual contribution is entirely Heberger's. What
`shed` adds is engineering reach: Heberger's delineator is built specifically around
the MERIT-Hydro and MERIT-Basins datasets, whereas `shed` runs the same method on any
[HFX](https://github.com/CooperBigFoot/hfx)-compliant hydrofabric — GRIT,
MERIT-Basins, and others. The algorithm is his; the fabric-agnostic generalization is
`shed`'s engineering delta. Because delineator is MIT-licensed, this reimplementation
is legally clean: the credit here is a matter of correctness and courtesy, not a
license obligation.

### A note on lineage

Heberger's own paper is candid about precedent, and so is this page. The idea of
combining raster and vector methods for watershed delineation traces back to
**Djokic and Ye (1999)**; Heberger's contribution is the modern open-source, free,
and global implementation that made the hybrid method fast and widely usable.

## How to cite Heberger's work

If the delineation algorithm matters to your work, please cite Heberger.

**The paper:**

> Heberger, M. (2025). Fast, accurate watershed delineation with a hybrid of raster and vector methods. Manuscript submitted to *Environmental Modelling & Software*. SSRN preprint: <https://doi.org/10.2139/ssrn.5939056> (<https://ssrn.com/abstract=5939056>). Author copy: [preprint PDF](https://mghydro.com/pages/Heberger_delineation_2025.pdf).

**The Global Watersheds web application:**

> Heberger, M. Global Watersheds (web application). <https://mghydro.com/watersheds>

**BibTeX for the delineator software:**

```bibtex
@software{delineator,
  author    = {Matthew Heberger},
  title     = {delineator: Global Watershed Delineation with Python},
  year      = {2026},
  publisher = {GitHub},
  version   = {2.1},
  url       = {https://github.com/mheberger/delineator}
}
```

## How to cite shed

`shed` and its Python bindings `pyshed` are MIT-licensed and do not have their own
DOI or paper. If you use them in research, cite Heberger's algorithm as above and
point to the repository:

> shed — watershed delineation for any HFX-compliant hydrofabric. <https://github.com/CooperBigFoot/shed>
