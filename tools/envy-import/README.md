# Envy area importer

`envy-import` is an offline, deliberately conservative bridge from stock EnvyMUD
2.2 area files to the versioned `WorldPack` interchange schema. Imported data is
never executed.

The first slice can:

- audit the canonical stock section sequence;
- parse the area name and room records with bounded byte readers;
- preserve raw room flags, sectors, exit values, and legacy VNUMs;
- resolve exits between rooms in the same file while retaining external VNUMs;
- emit validated JSON for later review and compilation.

It does not yet import mobiles, objects, resets, shops, or specials. In
particular, special procedure names are not code and will never be executed by
the importer.

```sh
cargo run --manifest-path tools/envy-import/Cargo.toml -- audit area.are
cargo run --manifest-path tools/envy-import/Cargo.toml -- \
  convert area.are --id example --encoding ascii > example.world.json
```

ASCII is the safe default for stock data. UTF-8 must be selected explicitly.
Other legacy encodings should be transcoded offline until the importer gains a
named, tested decoder for them.
