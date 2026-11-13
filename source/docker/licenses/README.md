# Locked third-party notices

From `source/`, run `node docker/licenses/generate.mjs --check`. It has no network
access, prints no upstream text, and fails if coverage, inputs, block hashes or
deterministic formatting differ. `THIRD_PARTY_NOTICES.md` is the sole storage for
upstream copyright/licence text. `evidence.json` stores identities and hashes.

Explicit collection: `node docker/licenses/generate.mjs --collect`. This requires
Node, tar, Docker with the exact Node/Rust base images already available, and
anonymous access to the exact locked package/source archives. Base image reads
use read-only, bounded containers with no network, volumes or published ports.
Source archive requests never use credentials. Collection always reads upstream;
no saved notice blocks are reused. Texts are decoded as strict UTF-8, falling
back to Latin-1 only when UTF-8 is invalid. Evidence records the raw bytes' hash,
encoding and any replacement character already present in the upstream file.
Each distinct text block is stored once with its applicable packages and files.
Review evidence and notices together before committing. Collection is never part
of CI or an image build. The Docker build checks the committed output instead.

After either generated file changes, run:

```sh
node docker/licenses/generate.mjs --verify-upstream --receipt /path/to/private/notices-receipt.json
```

This recollects with no reuse, compares both files byte for byte, and writes a
private receipt only on a match. The receipt contains `schemaVersion: 1`, raw
lowercase `noticesSha256` and `evidenceSha256`, counts and a UTC verification
time. Commit the receipt with those exact source blobs; preparation checks its
hashes without needing the network. A mismatching or incomplete collection must
not produce a receipt.

Ten exact package versions have explicitly approved licence evidence: three
crate entries elect Apache-2.0 and reuse the complete common licence, and seven
npm entries use the pinned SPDX standard text without its placeholder copyright
line. Their package declaration and stated author (or "none stated") precede
the standard text. Evidence marks `elected` or `spdx-standard`; no copyright line
is invented, and this is not proof of legal compliance.

`collectionFailures` lists unresolved evidence. A partial collection is not a
release-ready report: `--check` stays nonzero until every entry is covered.
A complete report records source/build dependencies conservatively, including
packages not shipped at runtime, and separately records runtime Debian packages
for both architectures. It does not replace a review of distribution obligations
or establish rights to the project's artwork.
