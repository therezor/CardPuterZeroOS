# The apt repo

`publish.sh` builds the signed site GitHub Pages serves at
`https://therezor.github.io/CardPuterZeroOS/apt`, and `.github/workflows/apt.yml`
runs it. `docs/updates.md` has the design: suites, the pool, Valid-Until.

## The key, once

An Ed25519 primary key stays offline; only a signing subkey goes to CI.

```sh
export GNUPGHOME=$(mktemp -d)
gpg --quick-gen-key "Gh0st OS archive <15324879+therezor@users.noreply.github.com>" ed25519 cert never
fpr=$(gpg --list-keys --with-colons | awk -F: '$1 == "fpr" {print $10; exit}')
gpg --quick-add-key "$fpr" ed25519 sign 2y
# Keep this backup offline: the primary key makes new subkeys.
gpg --armor --export-secret-keys "$fpr" > gh0st-archive-primary.asc
# The public key goes in the image, from this folder.
gpg --export --export-options export-minimal "$fpr" > os/repo/gh0st-archive-keyring.pgp
# The subkey alone, for the APT_SIGNING_KEY secret.
gpg --armor --export-secret-subkeys "$fpr" | gh secret set APT_SIGNING_KEY
```

Then, in the repo's settings, Pages > Source: GitHub Actions.

Commit `os/repo/gh0st-archive-keyring.pgp`. Images built before it
exists have no Gh0st OS source (`os/packages/gh0st-base/stage.sh`).

## By hand

```sh
os/build.sh fetch && os/build.sh debs
REPO_KEY="$(cat signing.asc)" os/build.sh publish edge   # os/out/pages/apt
REPO_PREV=https://therezor.github.io/CardPuterZeroOS/apt REPO_KEY=... os/build.sh publish edge
```

Without `REPO_PREV` the site starts empty. A subkey expires after two
years: make a new one with the primary key and replace the secret.
