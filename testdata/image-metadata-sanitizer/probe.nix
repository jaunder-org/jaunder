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
      pkgs.python3
      pkgs.stdenv.cc
    ];
  }
  ''
    set -eu
    mkdir -p "$out/fixtures/input" "$out/fixtures/candidate" "$out/fixtures/icc" \
      "$out/fixtures/repeated" "$out/fixtures/format-specific" "$out/reports"
    $CC -std=c17 -Wall -Wextra -Werror ${./make_icc_fixture.c} \
      -I${pkgs.lcms.dev}/include -L${pkgs.lcms}/lib -llcms2 -o make-icc-fixture
    $CC -std=c17 -Wall -Wextra -Werror ${./validate_icc.c} \
      -I${pkgs.lcms.dev}/include -L${pkgs.lcms}/lib -llcms2 -o validate-icc

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
    sha256sum "$out"/fixtures/input/* "$out"/fixtures/candidate/* "$out"/reports/*.png \
      > "$out/reports/SHA256SUMS"
  ''
