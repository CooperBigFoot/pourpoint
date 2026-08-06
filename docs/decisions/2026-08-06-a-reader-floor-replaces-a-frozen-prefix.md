# A reader floor replaces a frozen prefix when the deployed population is empty

The frozen-prefix discipline was adopted after the 2026-07-24 live fire, and it
was correct for what it was believed to protect. Declaring the planetary D8
entry in the live `grit/hfx-v0.3.0` manifest made every default GRIT carve on
released pourpoint 0.2.0 fail, the amendment was rolled back byte-identically
within ~25 minutes, and the rule written down afterwards was that a live prefix
never gains an entry a deployed released reader rejects — such an entry ships
under a successor prefix instead. `grit/hfx-v0.3.1` was fixed as that successor
in `RELEASING.md` on 2026-08-01.

That discipline buys exactly one thing: byte-stability for readers already
pointing at the prefix. Its value is therefore proportional to that population,
and at a population of zero it buys nothing while still costing a second
address, a second set of documentation pointers, and a version number that
distinguishes two byte-identical folders. On 2026-08-06 the repository owner
attested that pourpoint has no third-party installs and no GitHub stars. The
2026-08-01 keep-published decision rested on 37 and 36 installer-attributed
downloads that were never attributed to a person; the attestation is owner
recollection rather than a measurement, and is recorded as such rather than
dressed as evidence. Attributing those rows by country, operating system, and
Python version was offered and declined.

The decision is therefore to remove the straggler rather than route around it.
Released 0.2.0 and 0.2.1 are yanked; the planetary D8 entry is declared in place
in the `grit/hfx-v0.3.0` manifest; no successor prefix is minted, and
`grit/hfx-v0.3.1` is abandoned as a name. 0.2.0 is yanked alongside 0.2.1 rather
than left published as the loud-failure case, because its loudness is a property
of the planetary COGs' tile count and not of the release — it carries the same
ESRI-for-GRASS decode defect and lies just as quietly against a smaller
GRASS-declared raster. A yank is not a deletion: it changes what a version range
resolves to and leaves an exact pin installable, so it reaches future installs
and never an install already on disk. It suffices here only because the
attested installed base is the owner's own, and the record says so.

Ordering is load-bearing. The yank precedes the in-place declaration, because
declaring the entry while 0.2.0 remains installable reproduces the 2026-07-24
failure exactly. Nobody would be harmed at a population of zero, but the record
could not then honestly claim the hazard was gone at the moment of firing, and a
record that outruns its evidence is the failure this Program exists to stop.

The underlying rule — never change data under a reader already reading it — is
unchanged and is not what is being abandoned. What changes is how it is
enforced. Immobilising the data is replaced by stating a **reader floor**: the
lowest released version that reads a given address correctly, published beside
every address this repository hands out, with a mechanical check that fails when
an address is offered without one. This generalises past the case that prompted
it. The frozen prefix, #100's support claims, and this straggler were three
instances of one question — which readers may read this data — answered three
times from context held in one person's head. A floor answers it in the
repository. The floor is itself a support claim and inherits that term's
evidence discipline: the GRIT floor is 0.3.0 because the declaration-authority
and window-assembly fixes landed there, which is a derivation from the claim
catalog and not an observation of any reader reading that address. Turning it
into an observation belongs to the end-to-end verification effort, which fires
the declaration and proves the resulting geometry in one motion — splitting the
firing from the proving is what went wrong on 2026-07-24.

The costs are real, and paying them requires separating two kinds of prose that
mention the same names. A statement of *current state* becomes false the day
this lands and must be rewritten: two `CONTEXT.md` entries, and `RELEASING.md`
lines 125-165, whose whole section asserts the keep-published decision and the
successor mechanism in the present tense. A *dated record of what was observed
or decided at a past moment* stays exactly as written, because rewriting it
would destroy the evidence that the reversal happened and substitute a story in
which the absence of users was always known — that covers the 2026-08-01
keep-published decision itself, the 0.2.x entries in both changelogs, and
`docs/releases/tile-count-independent-planetary-cog-reads.md:242`, whose claim
that the telemetry was never written to the frozen prefix remains true of
2026-07-25 regardless of what happens to the prefix afterwards. `RELEASING.md`
already establishes the mechanism for this at lines 101-107, where a stream
status supersedes four historical status claims *without rewriting the packet
that carries them*; the same treatment applies here. The abandoned
`grit/hfx-v0.3.1` name is now recorded in more than one place as a decision made
and then reversed, and a future reader who finds only one of them will read a
contradiction. And the protection now rests on prose plus a check rather than on
data that cannot move, which is weaker in exchange for being general — a floor
that is stated and never verified is precisely the recorded-strength-exceeding-
evidence failure the floor is meant to prevent, so the check must fail against
the repository as it stands today before it is believed.
