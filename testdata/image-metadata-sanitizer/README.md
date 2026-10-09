# Owned artwork provenance

Only the small PNG/APNG, GIF and WebP artwork generators remain here. They use
Pillow and ordinary synthetic fixture construction; they are not production
sanitizers or shipping verification gates. They do not reconstruct every exact
host fixture filename or metadata-editing step.

The practical corpus, clean ExifTool golden outputs and conventional rendering
observer live under `host/src/image_sanitizer_fixtures/`. Its README records the
runtime invocation and preservation evidence.

The earlier custom codec/container parsers, rewriters, instrumented HEVC
consumer, ICC scrubbing experiments and `probe.nix` campaign driver were removed
when #1702 narrowed to conventional accidental-metadata protection. Historical
research documents describe those old experiments; their commands are archival,
not runnable shipping requirements. The old source remains in Git history.
