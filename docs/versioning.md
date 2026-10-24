# Versioning

Kubarr versions are tracked in the API, worker, and frontend package metadata:

- `code/api/Cargo.toml`
- `code/worker/Cargo.toml`
- `code/frontend/package.json`

Release builds are identified by git tags.

## Channels

- Stable: `v1.2.3`
- Release candidate: `v1.2.3-rc.1`
- Development: any untagged commit

## Manual Release

1. Update all three package versions and refresh their lockfiles.
2. Run API, worker, and frontend checks. Run the CI/CD workflow with `release-checks` enabled to validate live integration, acceptance, and VPN checks before tagging.
3. Commit the version changes.
4. Create an annotated tag.
5. Push the commit and tag.

Version-tag publishing requires the live integration, acceptance, and VPN jobs
to pass. The CLI has independent versions and releases in
[kubarr-cli](https://github.com/smokeythebandit/kubarr-cli). Platform chart
versions and the CLI's bootstrap pins must be coordinated with application
releases; an application tag does not update either repository automatically.

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
