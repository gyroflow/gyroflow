# Gyroflow tooling

## Lensfun profile import

`import_lensfun.py` converts Lensfun XML entries into Gyroflow lens profile JSON files for distortion models Gyroflow already supports.

Currently supported:

- Lensfun `ptlens` distortion entries → Gyroflow `distortion_model: "ptlens"`

Unsupported Lensfun distortion models are skipped deliberately so generated profiles do not silently use incorrect math.

Example:

```bash
python3 tools/import_lensfun.py /usr/share/lensfun/version_1 ./lens_profiles_from_lensfun
```

The generated JSON files can be reviewed and copied into a lens profile directory/bundle.
