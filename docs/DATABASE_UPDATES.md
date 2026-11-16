# Database-only releases

The Rust application embeds a usable game database, then optionally checks the
GitHub Releases API for a separate `database-<commit>` release. Database
updates do not replace or download the application binary. The main menu offers
**Check for Database Updates…** and **Restore Previous Database**.

## Release contents and validation

The `Database Release` workflow runs on pushes to `master` that change a JSON
file directly under `data/`. It publishes exactly two assets:

- `pop-fe-database-<12-character-commit>.zip`, containing the ten runtime JSON
  databases at the archive root.
- `database-manifest.json`, containing the format and schema versions, source
  commit, monotonic workflow sequence, archive SHA-256 and size, and each
  database file's SHA-256 and size.

The app accepts only the known format and schema and exact file allowlist. It
checks the release tag, archive size and checksum, each file's size and
checksum, UTF-8, JSON structure, and the complete archive path set before
installing. It writes into a new version directory and switches the active
pointer atomically only after validation. A failed download or validation
leaves the existing database available.

To prepare assets locally for inspection:

```sh
python3 tools/build_database_release.py \
  --revision <full-40-character-git-commit> \
  --sequence <positive-integer> \
  --output dist/database
```

The workflow supplies `GITHUB_SHA` and `GITHUB_RUN_NUMBER` automatically. To
publish a manual database release, run **Database Release** from the GitHub
Actions tab at the intended commit. Re-running it for a commit that already
has a release is a no-op; make a new commit for a corrected bundle.

## Updating and recovery

1. Choose **Main Menu → Check for Database Updates…**. The operation runs in
   the background; cancel requests are honored after the current network
   request completes and before installation begins.
2. Restart the application after a successful update. The database is loaded
   once at startup so that an operation cannot change data halfway through.
3. If the new data causes a problem, choose **Main Menu → Restore Previous
   Database**, then restart. After the first update, this restores the bundled
   database; later updates restore the preceding installed release.

The installed active and previous versions are stored under
`<user-config-directory>/org.popfe.PSPManager/database-updates/`. The app keeps
versioned copies so a failed activation cannot overwrite the prior database.
If no update has ever been installed, or the network/GitHub is unavailable,
the app continues with its embedded data. A local rollback only changes the
active database pointer; it does not change or delete a GitHub release.

## Release rollback

Do not replace or delete a published database asset. To roll back data for
everyone, revert the database source JSON changes in a new commit on `master`.
The workflow will publish a new monotonically sequenced database release from
that commit. Existing clients can then install it without downloading a new
application package. The in-app previous-version action remains available for
local recovery.
