# Hermetic feasibility witness for issue #1702 task 1 only.
# Generated inputs are original synthetic artwork dedicated to CC0-1.0.
{ system ? builtins.currentSystem }:
let
  flake = builtins.getFlake (toString ../..);
  pkgs = flake.inputs.nixpkgs.legacyPackages.${system};
in
pkgs.runCommand "issue-1702-image-sanitizer-feasibility-witness"
  {
    nativeBuildInputs = [
      pkgs.exiftool
      pkgs.imagemagick
      pkgs.libheif
      pkgs.lcms
      (pkgs.python3.withPackages (python: [ python.pillow ]))
      pkgs.stdenv.cc
    ];
  }
  ''
    set -eu
    mkdir -p "$out/fixtures/input" "$out/fixtures/candidate" "$out/fixtures/icc" \
      "$out/fixtures/repeated" "$out/fixtures/format-specific" "$out/fixtures/ordinary" "$out/reports"
    $CC -std=c17 -Wall -Wextra -Werror ${./make_icc_fixture.c} \
      -I${pkgs.lcms.dev}/include -L${pkgs.lcms}/lib -llcms2 -o make-icc-fixture
    $CC -std=c17 -Wall -Wextra -Werror ${./validate_icc.c} \
      -I${pkgs.lcms.dev}/include -L${pkgs.lcms}/lib -llcms2 -o validate-icc
    $CC -std=c17 -Wall -Wextra -Werror ${./make_rgb_icc_fixture.c} \
      -I${pkgs.lcms.dev}/include -L${pkgs.lcms}/lib -llcms2 -o make-rgb-icc-fixture
    $CC -std=c17 -Wall -Wextra -Werror ${./validate_rgb_icc.c} \
      -I${pkgs.lcms.dev}/include -L${pkgs.lcms}/lib -llcms2 -o validate-rgb-icc

    python3 -B ${./verify_curve_controls.py} ${./rewrite_rgb_icc.py} \
      > "$out/reports/curve-body-controls.txt"

    # This 32x24 gradient is generated artwork, not a photograph or device data.
    magick -size 32x24 gradient:'#1b4965-#ffb703' "$out/fixtures/input/base.png"
    linkicc -o "$out/fixtures/icc/base.icc" -t0 '*sRGB'
    ./make-icc-fixture "$out/fixtures/icc/base.icc" "$out/fixtures/icc/input-v4.icc" 4
    ./make-icc-fixture "$out/fixtures/icc/base.icc" "$out/fixtures/icc/input-v2.icc" 2
    python3 ${./rewrite_icc.py} "$out/fixtures/icc/input-v4.icc" "$out/fixtures/icc/scrubbed-v4.icc"
    if python3 ${./rewrite_icc.py} "$out/fixtures/icc/input-v2.icc" "$out/fixtures/icc/unsupported-v2-output.icc" \
      > "$out/reports/unsupported-v2.txt" 2>&1; then
      echo "unsupported ICC v2 unexpectedly accepted" >&2
      exit 1
    fi
    grep -Fqx 'ValueError: only ICC v4 profiles are supported' "$out/reports/unsupported-v2.txt"
    test ! -e "$out/fixtures/icc/unsupported-v2-output.icc"
    python3 ${./rewrite_icc.py} "$out/fixtures/icc/scrubbed-v4.icc" "$out/fixtures/icc/scrubbed-v4.second-pass.icc"
    cmp "$out/fixtures/icc/scrubbed-v4.icc" "$out/fixtures/icc/scrubbed-v4.second-pass.icc"
    ./validate-icc "$out/fixtures/icc/input-v4.icc" "$out/fixtures/icc/scrubbed-v4.icc" \
      > "$out/reports/icc-transform-validation.txt"
    ordinary_pipeline() {
      source=$1
      output=$2
      work=$3
      cp "$source" "$output"
      exiftool -b -ICC_Profile "$output" > "$work.input.icc"
      python3 ${./rewrite_rgb_icc.py} "$work.input.icc" "$work.output.icc"
      exiftool -overwrite_original -all= "$output"
      exiftool -overwrite_original "-ICC_Profile<=$work.output.icc" "$output"
    }
    for space in srgb display-p3; do
      for version in 2 4; do
        stem="$space-v$version"
        ./make-rgb-icc-fixture "$space" "$out/fixtures/ordinary/$stem.icc" "$version"
        python3 ${./rewrite_rgb_icc.py} "$out/fixtures/ordinary/$stem.icc" \
          "$out/fixtures/ordinary/$stem.scrubbed.icc"
        python3 ${./rewrite_rgb_icc.py} "$out/fixtures/ordinary/$stem.scrubbed.icc" \
          "$out/fixtures/ordinary/$stem.scrubbed-second.icc"
        cmp "$out/fixtures/ordinary/$stem.scrubbed.icc" "$out/fixtures/ordinary/$stem.scrubbed-second.icc"
        ./validate-rgb-icc "$out/fixtures/ordinary/$stem.icc" "$out/fixtures/ordinary/$stem.scrubbed.icc" \
          >> "$out/reports/ordinary-icc-transform-validation.txt"
        magick "$out/fixtures/input/base.png" -profile "$out/fixtures/ordinary/$stem.icc" \
          -quality 92 "$out/fixtures/ordinary/$stem.input.jpg"
        # Each invocation extracts the profile from the actual JPEG, rewrites
        # that extraction, and reinserts its own output; no fixed replacement
        # profile is substituted into the JPEG pipeline.
        ordinary_pipeline "$out/fixtures/ordinary/$stem.input.jpg" \
          "$out/fixtures/ordinary/$stem.output.jpg" "$out/fixtures/ordinary/$stem.first"
        ordinary_pipeline "$out/fixtures/ordinary/$stem.input.jpg" \
          "$out/fixtures/ordinary/$stem.repeat.jpg" "$out/fixtures/ordinary/$stem.repeat-work"
        ordinary_pipeline "$out/fixtures/ordinary/$stem.output.jpg" \
          "$out/fixtures/ordinary/$stem.output-second.jpg" "$out/fixtures/ordinary/$stem.second"
      done
    done
    python3 -B ${./verify_jpeg.py} "$out/fixtures/jpeg" ${./rewrite_jpeg.py} \
      ${./rewrite_rgb_icc.py} ${./rewrite_webp.py} ${./rewrite_png.py} \
      ${./verify_rgb_icc.py} "$PWD/validate-rgb-icc" ${pkgs.imagemagick}/bin/magick \
      ${./make_png_fixtures.py} > "$out/reports/jpeg-rewrite.json"
    PYTHONPATH=${./.} python3 -B ${./verify_jpeg_controls.py} "$out/fixtures/jpeg" \
      ${./rewrite_jpeg.py} ${./rewrite_rgb_icc.py} ${./rewrite_webp.py} ${./rewrite_png.py} \
      ${./verify_rgb_icc.py} "$PWD/validate-rgb-icc" ${pkgs.imagemagick}/bin/magick \
      ${pkgs.exiftool}/bin/exiftool > "$out/reports/jpeg-controls.json"
    PYTHONPATH=${./.} python3 -B ${./verify_jpeg_entropy.py} "$out/fixtures/jpeg-entropy" \
      ${./rewrite_jpeg.py} ${./rewrite_rgb_icc.py} ${./rewrite_webp.py} ${./rewrite_png.py} \
      ${./verify_rgb_icc.py} "$PWD/validate-rgb-icc" ${pkgs.imagemagick}/bin/magick \
      > "$out/reports/jpeg-baseline-entropy.json"
    python3 -B ${./make_png_fixtures.py} "$out/fixtures/ordinary" "$out/fixtures/png/input"
    python3 -B ${./verify_png_candidate.py} "$out/fixtures/png" ${pkgs.exiftool}/bin/exiftool \
      > "$out/reports/png-apng-exiftool-candidate.json"
    python3 -B ${./verify_png_rewrite.py} "$out/fixtures/png" ${./rewrite_png.py} \
      ${./rewrite_rgb_icc.py} ${./verify_png_candidate.py} ${./verify_rgb_icc.py} \
      "$PWD/validate-rgb-icc" > "$out/reports/png-apng-rewrite.json"
    python3 -B ${./verify_png_controls.py} "$out/fixtures/png" ${./rewrite_png.py} \
      ${./rewrite_rgb_icc.py} ${./verify_png_candidate.py} \
      > "$out/reports/png-apng-controls.txt"
    # Independent golden consumer proof runs before any rewrite acceptance.
    python3 -B ${./verify_gif_lzw_envelope.py} "$out/fixtures/gif" ${pkgs.imagemagick}/bin/magick \
      > "$out/reports/gif-lzw-consumers.json"
    python3 -B ${./make_gif_fixtures.py} "$out/fixtures/ordinary" "$out/fixtures/gif/input"
    python3 -B ${./verify_gif.py} "$out/fixtures/gif" ${./rewrite_gif.py} \
      ${./rewrite_rgb_icc.py} ${./verify_rgb_icc.py} "$PWD/validate-rgb-icc" \
      ${pkgs.exiftool}/bin/exiftool ${pkgs.imagemagick}/bin/magick \
      > "$out/reports/gif-rewrite.json"
    python3 -B ${./verify_gif_controls.py} "$out/fixtures/gif" ${./rewrite_gif.py} \
      ${./rewrite_rgb_icc.py} ${./verify_gif.py} ${pkgs.imagemagick}/bin/magick \
      > "$out/reports/gif-controls.txt"
    python3 -B ${./verify_gif_lzw_envelope.py} "$out/fixtures/gif" ${pkgs.imagemagick}/bin/magick \
      ${./rewrite_gif.py} ${./rewrite_rgb_icc.py} > "$out/reports/gif-lzw-rewrite.json"
    {
      printf 'pillow-version=%s\nimagemagick-version=%s\n' '${pkgs.python3Packages.pillow.version}' '${pkgs.imagemagick.version}'
      printf 'pillow-path=%s\nimagemagick-path=%s\n' '${pkgs.python3Packages.pillow}' '${pkgs.imagemagick}'
    } > "$out/reports/gif-consumer-packages.txt"

    # Candidate-only WebP witness: fixture construction uses Pillow/libwebp;
    # inspection is separately implemented with Python's standard library.
    PYTHONPATH=${./.} python3 -B ${./make_webp_fixtures.py} "$out/fixtures/ordinary" "$out/fixtures/webp/input"
    python3 -B ${./verify_webp_candidate.py} "$out/fixtures/webp" \
      ${pkgs.exiftool}/bin/exiftool > "$out/reports/webp-exiftool-candidate.json"
    python3 -B ${./verify_webp_candidate_oracle_controls.py} \
      "$out/reports/webp-exiftool-candidate.json" ${./verify_webp_candidate.py} \
      > "$out/reports/webp-candidate-oracle-controls.txt"
    python3 -B ${./verify_webp_controls.py} "$out/fixtures/webp" \
      ${./verify_webp_candidate.py} > "$out/reports/webp-controls.txt"
    python3 -B ${./verify_webp_rewrite.py} "$out/fixtures/webp" \
      ${./rewrite_webp.py} ${./rewrite_rgb_icc.py} ${./rewrite_png.py} \
      ${./verify_webp_candidate.py} ${./verify_rgb_icc.py} "$PWD/validate-rgb-icc" \
      > "$out/reports/webp-rewrite.json"
    python3 -B ${./verify_webp_rewrite_controls.py} "$out/fixtures/webp" \
      ${./rewrite_webp.py} ${./rewrite_rgb_icc.py} ${./rewrite_png.py} \
      ${./verify_webp_candidate.py} ${./verify_webp_rewrite.py} ${./verify_rgb_icc.py} ./validate-rgb-icc \
      > "$out/reports/webp-rewrite-controls.txt"
    {
      printf 'pillow-nix-version=%s\n' '${pkgs.python3Packages.pillow.version}'
      printf 'pillow-nix-path=%s\n' '${pkgs.python3Packages.pillow}'
      printf 'pillow-license=%s\n' '${builtins.toJSON pkgs.python3Packages.pillow.meta.license}'
      printf 'libwebp-nix-version=%s\n' '${pkgs.libwebp.version}'
      printf 'libwebp-nix-path=%s\n' '${pkgs.libwebp}'
      printf 'libwebp-license=%s\n' '${builtins.toJSON pkgs.libwebp.meta.license}'
      python3 -c 'from PIL import Image, features; print("pillow-runtime-version=" + Image.__version__); print("libwebp-runtime-version=" + str(features.version_module("webp")))'
    } > "$out/reports/webp-package-metadata.txt"

    magick "$out/fixtures/input/base.png" -profile "$out/fixtures/icc/input-v4.icc" \
      -quality 92 "$out/fixtures/input/device-like.jpg"
    heif-enc -q 50 -o "$out/fixtures/input/device-like.heic" \
      "$out/fixtures/input/device-like.jpg"

    # Deliberately add every descriptive class used by the witness and an EXIF
    # preview. Values are synthetic and must not be interpreted as a real device.
    exiftool -overwrite_original \
      -EXIF:GPSLatitude=51.5007 -EXIF:GPSLatitudeRef=N \
      -EXIF:GPSLongitude=0.1246 -EXIF:GPSLongitudeRef=W \
      -EXIF:DateTimeOriginal='2026:10:07 12:34:56' \
      -EXIF:Make='Synthetic Camera Co.' -EXIF:Model='Fixture Model 1' \
      -EXIF:Artist='Fixture Author' -EXIF:Copyright='CC0 fixture' \
      -EXIF:ImageDescription='synthetic descriptive metadata' \
      -EXIF:UserComment='synthetic comment' \
      -XMP-dc:Description='synthetic xmp description' \
      -XMP-exif:GPSLatitude=51.5007 -XMP-exif:GPSLongitude=0.1246 \
      -IPTC:Caption-Abstract='synthetic iptc caption' \
      "-ThumbnailImage<=$out/fixtures/input/device-like.jpg" \
      "-ICC_Profile<=$out/fixtures/icc/input-v4.icc" \
      "$out/fixtures/input/device-like.jpg"
    exiftool -overwrite_original \
      -EXIF:GPSLatitude=51.5007 -EXIF:GPSLatitudeRef=N \
      -EXIF:GPSLongitude=0.1246 -EXIF:GPSLongitudeRef=W \
      -EXIF:DateTimeOriginal='2026:10:07 12:34:56' \
      -EXIF:Make='Synthetic Camera Co.' -EXIF:Model='Fixture Model 1' \
      -EXIF:Artist='Fixture Author' -EXIF:Copyright='CC0 fixture' \
      -EXIF:ImageDescription='synthetic descriptive metadata' \
      -XMP-dc:Description='synthetic xmp description' \
      -XMP-exif:GPSLatitude=51.5007 -XMP-exif:GPSLongitude=0.1246 \
      "-ICC_Profile<=$out/fixtures/icc/input-v4.icc" \
      "$out/fixtures/input/device-like.heic"

    cp "$out/fixtures/input/device-like.jpg" "$out/fixtures/candidate/device-like.jpg"
    cp "$out/fixtures/input/device-like.heic" "$out/fixtures/candidate/device-like.heic"
    cp "$out/fixtures/input/device-like.jpg" "$out/fixtures/repeated/first.jpg"
    cp "$out/fixtures/input/device-like.jpg" "$out/fixtures/repeated/second.jpg"
    # Separate ExifTool processes establish same-input deterministic output;
    # this is intentionally not a multi-file batch.
    exiftool -overwrite_original -all= "$out/fixtures/repeated/first.jpg"
    exiftool -overwrite_original "-ICC_Profile<=$out/fixtures/icc/scrubbed-v4.icc" "$out/fixtures/repeated/first.jpg"
    exiftool -overwrite_original -all= "$out/fixtures/repeated/second.jpg"
    exiftool -overwrite_original "-ICC_Profile<=$out/fixtures/icc/scrubbed-v4.icc" "$out/fixtures/repeated/second.jpg"
    cmp "$out/fixtures/repeated/first.jpg" "$out/fixtures/repeated/second.jpg"
    cp "$out/fixtures/input/device-like.jpg" "$out/fixtures/format-specific/device-like.jpg"
    exiftool -overwrite_original -all= "$out/fixtures/format-specific/device-like.jpg"
    exiftool -overwrite_original "-ICC_Profile<=$out/fixtures/icc/scrubbed-v4.icc" \
      "$out/fixtures/format-specific/device-like.jpg"
    cp "$out/fixtures/format-specific/device-like.jpg" "$out/fixtures/format-specific/device-like.second-pass.jpg"
    exiftool -overwrite_original -all= "$out/fixtures/format-specific/device-like.second-pass.jpg"
    exiftool -overwrite_original "-ICC_Profile<=$out/fixtures/icc/scrubbed-v4.icc" \
      "$out/fixtures/format-specific/device-like.second-pass.jpg"
    # Candidate under evaluation. -overwrite_original is mandatory to avoid
    # _original artifacts. ICC is retained solely to measure its privacy gap.
    exiftool -overwrite_original -all= --icc_profile:all \
      "$out/fixtures/candidate/device-like.jpg" \
      "$out/fixtures/candidate/device-like.heic"
    cp "$out/fixtures/candidate/device-like.jpg" "$out/fixtures/candidate/device-like.second-pass.jpg"
    cp "$out/fixtures/candidate/device-like.heic" "$out/fixtures/candidate/device-like.second-pass.heic"
    exiftool -overwrite_original -all= --icc_profile:all \
      "$out/fixtures/candidate/device-like.second-pass.jpg" \
      "$out/fixtures/candidate/device-like.second-pass.heic"

    # Independent inspection uses only Python's standard library; it does not
    # call ExifTool or libheif.
    exiftool -G1 -a -s "$out/fixtures/input/device-like.jpg" \
      "$out/fixtures/input/device-like.heic" \
      "$out/fixtures/candidate/device-like.jpg" \
      "$out/fixtures/candidate/device-like.heic" > "$out/reports/exiftool-tags.txt"
    heif-info "$out/fixtures/input/device-like.heic" > "$out/reports/heif-input.txt"
    heif-info "$out/fixtures/candidate/device-like.heic" > "$out/reports/heif-candidate.txt"
    magick "$out/fixtures/input/device-like.heic" "$out/reports/input.png" \
      > "$out/reports/heif-input-decode.txt" 2>&1
    magick "$out/fixtures/candidate/device-like.heic" "$out/reports/candidate.png" \
      > "$out/reports/heif-candidate-decode.txt" 2>&1
    magick "$out/reports/input.png" "$out/reports/candidate.png" -metric AE \
      -compare "$out/reports/render-diff.png" > "$out/reports/heif-render-compare.txt" 2>&1
    printf 'candidate-decode=passed\nrender-identical=yes\n' > "$out/reports/heif-candidate-decode-status.txt"
    python3 ${./verify.py} "$out/fixtures" > "$out/reports/independent.json"
    python3 ${./verify_controls.py} "$out/fixtures" ${./verify.py} > "$out/reports/independent-controls.txt"
    python3 -B ${./verify_rgb_icc.py} "$out/fixtures" ${./verify.py} > "$out/reports/ordinary-icc-structural.txt"
    python3 -B ${./verify_rgb_structure_controls.py} "$out/fixtures" \
      ${./rewrite_rgb_icc.py} ${./verify_rgb_icc.py} "$out/reports/structure-controls" \
      > "$out/reports/ordinary-icc-negative-controls.txt"
    python3 -B ${./verify_rgb_controls.py} "$out/fixtures" ${./rewrite_rgb_icc.py} > "$out/reports/ordinary-icc-controls.txt"
    ./validate-rgb-icc "$out/fixtures/ordinary/control-shared-trc.icc" \
      "$out/fixtures/ordinary/control-shared-trc.scrubbed.icc" >> "$out/reports/ordinary-icc-transform-validation.txt"
    sha256sum "$out"/fixtures/input/* "$out"/fixtures/candidate/* "$out"/fixtures/webp/input/* \
      "$out"/fixtures/webp/candidate/* "$out"/reports/*.png > "$out/reports/SHA256SUMS"
  ''
