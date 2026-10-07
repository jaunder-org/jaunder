/* Independent ordinary-RGB ICC transform equivalence test via LittleCMS. */
#include <lcms2.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static void fail(const char *message) {
  fputs(message, stderr);
  fputc('\n', stderr);
  exit(1);
}

static cmsHTRANSFORM open_transform(const char *path, cmsUInt32Number intent) {
  cmsHPROFILE profile = cmsOpenProfileFromFile(path, "r");
  cmsHTRANSFORM transform;
  if (profile == NULL) {
    fail("LittleCMS could not open ordinary RGB profile");
  }
  transform = cmsCreateTransform(profile, TYPE_RGB_8, NULL, TYPE_XYZ_16, intent, 0);
  cmsCloseProfile(profile);
  if (transform == NULL) {
    fail("LittleCMS could not create ordinary RGB transform");
  }
  return transform;
}

int main(int argc, char **argv) {
  const cmsUInt32Number intents[] = {INTENT_PERCEPTUAL, INTENT_RELATIVE_COLORIMETRIC,
                                     INTENT_SATURATION, INTENT_ABSOLUTE_COLORIMETRIC};
  uint8_t rgb[3];
  uint16_t left_xyz[3];
  uint16_t right_xyz[3];
  int intent;
  int r;
  int g;
  int b;

  if (argc != 3) {
    fail("usage: validate_rgb_icc INPUT OUTPUT");
  }
  for (intent = 0; intent < 4; intent++) {
    cmsHTRANSFORM left = open_transform(argv[1], intents[intent]);
    cmsHTRANSFORM right = open_transform(argv[2], intents[intent]);
    for (r = 0; r < 256; r += 17) {
      for (g = 0; g < 256; g += 17) {
        for (b = 0; b < 256; b += 17) {
          rgb[0] = (uint8_t)r;
          rgb[1] = (uint8_t)g;
          rgb[2] = (uint8_t)b;
          cmsDoTransform(left, rgb, left_xyz, 1);
          cmsDoTransform(right, rgb, right_xyz, 1);
          if (left_xyz[0] != right_xyz[0] || left_xyz[1] != right_xyz[1] ||
              left_xyz[2] != right_xyz[2]) {
            fail("ordinary RGB colour transform changed");
          }
        }
      }
    }
    cmsDeleteTransform(left);
    cmsDeleteTransform(right);
  }
  puts("LittleCMS ordinary RGB equivalence passed for 4 intents x 4,096 samples");
  return 0;
}
