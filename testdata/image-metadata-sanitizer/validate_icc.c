/* Independent LittleCMS transform equivalence check for the fixture-scoped probe. */
#include <lcms2.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static void fail(const char *message) {
  fputs(message, stderr);
  fputc('\n', stderr);
  exit(1);
}

static cmsHTRANSFORM open_transform(const char *path) {
  cmsHPROFILE profile = cmsOpenProfileFromFile(path, "r");
  cmsHTRANSFORM transform;
  if (profile == NULL) {
    fail("LittleCMS could not open profile");
  }
  transform = cmsCreateTransform(profile, TYPE_RGB_8, NULL, TYPE_XYZ_16, INTENT_PERCEPTUAL, 0);
  cmsCloseProfile(profile);
  if (transform == NULL) {
    fail("LittleCMS could not create RGB-to-XYZ transform");
  }
  return transform;
}

int main(int argc, char **argv) {
  cmsHTRANSFORM before;
  cmsHTRANSFORM after;
  uint8_t rgb[3];
  uint16_t before_xyz[3];
  uint16_t after_xyz[3];
  int r;
  int g;
  int b;

  if (argc != 3) {
    fail("usage: validate_icc INPUT OUTPUT");
  }
  before = open_transform(argv[1]);
  after = open_transform(argv[2]);
  for (r = 0; r < 256; r += 17) {
    for (g = 0; g < 256; g += 17) {
      for (b = 0; b < 256; b += 17) {
        rgb[0] = (uint8_t)r;
        rgb[1] = (uint8_t)g;
        rgb[2] = (uint8_t)b;
        cmsDoTransform(before, rgb, before_xyz, 1);
        cmsDoTransform(after, rgb, after_xyz, 1);
        if (before_xyz[0] != after_xyz[0] || before_xyz[1] != after_xyz[1] ||
            before_xyz[2] != after_xyz[2]) {
          fail("colour transform changed");
        }
      }
    }
  }
  cmsDeleteTransform(before);
  cmsDeleteTransform(after);
  puts("LittleCMS RGB-to-XYZ transform equivalence passed for 4,096 samples");
  return 0;
}
