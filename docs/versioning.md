# Versioning

Kubarr versions are tracked in the backend and frontend package metadata:

- `code/backend/Cargo.toml`
- `code/frontend/package.json`

Release builds are identified by git tags.

## Channels

- Stable: `v1.2.3`
- Release candidate: `v1.2.3-rc.1`
- Development: any untagged commit

## Manual Release

1. Update both package versions.
2. Run backend and frontend checks.
3. Commit the version changes.
4. Create an annotated tag.
5. Push the commit and tag.

```bash
git tag -a v1.2.3 -m "Release 1.2.3"
git push
git push origin v1.2.3
```

For release candidates:

```bash
git tag -a v1.2.3-rc.1 -m "Release candidate 1.2.3-rc.1"
git push
git push origin v1.2.3-rc.1
```
