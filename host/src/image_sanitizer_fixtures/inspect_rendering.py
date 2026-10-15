"""Small owned-fixture observer using conventional decoders, not container parsing."""
import hashlib
import json
import subprocess
import sys
from PIL import Image, UnidentifiedImageError

path, magick, exiftool = sys.argv[1:]
try:
    with Image.open(path) as image:
        result = {"frames": [], "loop": image.info.get("loop"),
                  "default_image": image.info.get("default_image", False)}
        for index in range(getattr(image, "n_frames", 1)):
            image.seek(index)
            rgba = image.convert("RGBA")
            result["frames"].append({"size": rgba.size,
                "rgba_sha256": hashlib.sha256(rgba.tobytes()).hexdigest(),
                "alpha_extrema": rgba.getchannel("A").getextrema(),
                "duration": image.info.get("duration"),
                "disposal": image.info.get("disposal"),
                "blend": image.info.get("blend")})
except UnidentifiedImageError:
    # Pillow has no HEIF decoder in this pinned closure; use libheif through
    # ImageMagick. These owned HEIC fixtures are static.
    size = subprocess.run([magick, "identify", "-format", "%w,%h", path],
                          capture_output=True, check=True, timeout=20).stdout.decode()
    pixels = subprocess.run([magick, path, "-depth", "8", "rgba:-"],
                            capture_output=True, check=True, timeout=20).stdout
    result = {"size": size, "rgba_sha256": hashlib.sha256(pixels).hexdigest()}
profile = subprocess.run([exiftool, "-config", "", "-b", "-ICC_Profile", path],
                         capture_output=True, check=True, timeout=20).stdout
result["icc_profile"] = {"length": len(profile), "sha256": hashlib.sha256(profile).hexdigest()}
print(json.dumps(result, sort_keys=True))
