# Historical EnvyMUD/Merc area importer

`envy-import` is Tiamud's offline, deliberately conservative bridge from
historical EnvyMUD/Merc-family area files to the versioned `WorldPack`
interchange schema. The currently tested subset targets stock EnvyMUD 2.2 area
structure. Imported data is never executed. The tool keeps its format-specific
name for historical compatibility; it is not the product name.

Tiamud is the definitive product. EnvyMUD, Merc, and other classic text MUDs
are acknowledged only as historical inspirations and compatibility sources;
none of them is the runtime engine or canonical content format.

The implemented importer can:

- audit the canonical stock section sequence;
- preserve bounded help records as inert, review-required compatibility data;
- parse area metadata/recall, mobiles, objects, rooms, resets, shops,
  specials, and inline or external MOB/OBJ/ROOM programs with bounded readers;
- preserve raw room flags, sectors, exit values, and legacy VNUMs;
- resolve exits between rooms in the same file while retaining external VNUMs;
- convert only unambiguous unconditional room-object resets into distinct
  WorldPack item instances;
- preserve every other historical construct as inert structured data with
  line/VNUM diagnostics and recommendations;
- reject ambiguous duplicate room VNUMs, exit directions, shop keepers,
  special assignments, and repeated supported extension sections;
- diagnose unknown sections/descriptors and external or broken exit, reset,
  shop, special, program-owner, and recall references without guessing;
- emit either validated WorldPack JSON or a complete machine-readable report.

WorldPack v2 cannot yet represent NPCs, shops, conditional resets, containers,
equipment, door state, or historical programs. The importer never guesses:
those records remain in `LegacyContent`, and special procedure/program text is
never executed or copied into the runtime WorldPack.

```sh
cargo run --manifest-path tools/envy-import/Cargo.toml -- audit area.are
cargo run --manifest-path tools/envy-import/Cargo.toml -- \
  convert area.are --id example --encoding ascii > example.world.json
cargo run --manifest-path tools/envy-import/Cargo.toml -- \
  convert-report area.are --id example --encoding ascii > example.import-report.json
```

ASCII is the safe default for stock data. UTF-8 must be selected explicitly.
Other legacy encodings should be transcoded offline until the importer gains a
named, tested decoder for them. Conversion validates the whole file before
parsing and reports the offending byte offset, line, and column. A UTF-8 BOM is
rejected because stock files must begin directly with `#AREA`.

Historical help records are parsed and included in `LegacyContent`, but never
published into a WorldPack automatically because formats, authorship and
licenses vary substantially. Each record carries an actionable review
diagnostic. `convert-report` also records the source path (when supplied),
format, audited dialect, selected text encoding, byte length, and SHA-256
digest. Its license status is deliberately
`undetermined_review_required`: the tool cannot infer ownership or grant
distribution rights, so a human must record authoritative provenance and make
the license decision before publication.

An unknown named section is reported and ignored/preserved only where the
stock parser can still delimit the audited sections safely. A duplicate
supported extension section is a hard audit error, since accepting only the
first copy would lose data silently. Numeric flags and descriptors remain raw;
diagnostics identify values whose runtime meaning still requires comparison
with the exact historical codebase.
