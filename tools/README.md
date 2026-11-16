# Rust data maintenance

The application embeds generated JSON databases from `data/`. These scripts
remain Python because they maintain Rust application data; they are not part
of the runtime application.

- `export_rust_data.py` reads the source datasets in `db.py` and `gamedb.py`
  and regenerates the JSON files consumed by `src/database.rs`. The PSP binary
  templates under `data/psp/` are checked-in source assets and are not generated
  by this script.
- `import_libcrypt_patches.py` imports unambiguous compatibility patch mappings
  from an upstream `libcrypt-patcher` checkout into Rust's embedded data.
- `build_database_release.py` packages the ten runtime JSON databases into a
  versioned ZIP with SHA-256 checksums and a manifest. GitHub Actions publishes
  these assets as database-only releases; see [database update operations](../docs/DATABASE_UPDATES.md).

Run the exporter from any working directory with Python 3:

```sh
python3 tools/export_rust_data.py
```
