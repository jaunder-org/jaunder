/* Generates owned ordinary RGB monitor ICC fixtures with pinned LittleCMS. */
#include <lcms2.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void fail(const char *message) {
  fputs(message, stderr);
  fputc('\n', stderr);
  exit(1);
}

static cmsUInt32Number read_u32_at(FILE *file, long position) {
  unsigned char bytes[4];
  if (fseek(file, position, SEEK_SET) != 0 || fread(bytes, 1, 4, file) != 4) {
    fail("could not read owned fixture field");
  }
  return ((cmsUInt32Number)bytes[0] << 24) | ((cmsUInt32Number)bytes[1] << 16) |
         ((cmsUInt32Number)bytes[2] << 8) | bytes[3];
}

/* LittleCMS 2.19.1 counts alignment in its legacy desc extent. ICC 7.2
 * excludes it. Correct ONLY our generated fixture's table size, leaving the
 * actual bytes/offsets and every transform tag untouched. Not an input repair
 * path for the sanitizer: improperly enlarged user tag sizes still reject. */
static void normalize_owned_v2_description_extent(const char *path) {
  FILE *file = fopen(path, "r+b");
  if (file == NULL) {
    fail("could not open owned v2 fixture");
  }
  cmsUInt32Number count = read_u32_at(file, 128);
  if (count != 11) {
    fail("unexpected owned fixture tag count");
  }
  for (cmsUInt32Number index = 0; index < count; index++) {
    long entry = 132 + 12 * index;
    if (read_u32_at(file, entry) != 0x64657363) {
      continue;
    }
    cmsUInt32Number offset = read_u32_at(file, entry + 4);
    cmsUInt32Number size = read_u32_at(file, entry + 8);
    cmsUInt32Number ascii_count = read_u32_at(file, offset + 8);
    if (ascii_count < 1 || ascii_count > 80) {
      fail("unexpected owned description ASCII count");
    }
    cmsUInt32Number unicode_count = read_u32_at(file, offset + 16 + ascii_count);
    if (unicode_count != ascii_count) {
      fail("unexpected owned description Unicode count");
    }
    cmsUInt32Number used = 12 + ascii_count + 8 + 2 * unicode_count + 70;
    if (size != ((used + 3) & ~3U)) {
      fail("unexpected owned description extent size");
    }
    for (cmsUInt32Number position = used; position < size; position++) {
      if (fseek(file, offset + position, SEEK_SET) != 0 || fgetc(file) != 0) {
        fail("owned description has nonzero alignment bytes");
      }
    }
    unsigned char bytes[4] = {used >> 24, used >> 16, used >> 8, used};
    if (fseek(file, entry + 8, SEEK_SET) != 0 || fwrite(bytes, 1, 4, file) != 4 || fclose(file) != 0) {
      fail("could not normalize owned description extent");
    }
    return;
  }
  fail("owned v2 fixture lacks description");
}

static void stamp_header(const char *path) {
  FILE *file = fopen(path, "r+b");
  /* 2026-10-07 12:34:56: synthetic source metadata for scrub tests. */
  unsigned char date[12] = {7, 234, 0, 10, 0, 7, 0, 12, 0, 34, 0, 56};
  if (file == NULL || fseek(file, 24, SEEK_SET) != 0 || fwrite(date, 1, sizeof(date), file) != sizeof(date) ||
      fseek(file, 48, SEEK_SET) != 0 || fwrite("JAUDTEST", 1, 8, file) != 8 ||
      fseek(file, 80, SEEK_SET) != 0 || fwrite("OWND", 1, 4, file) != 4 || fclose(file) != 0) {
    fail("could not stamp synthetic RGB profile header");
  }
}

static cmsMLU *labels(const char *space) {
  cmsMLU *value = cmsMLUalloc(NULL, 2);
  char english[80];
  char french[80];
  if (value == NULL) {
    fail("could not allocate RGB profile labels");
  }
  snprintf(english, sizeof(english), "Owned %s display profile", space);
  snprintf(french, sizeof(french), "Profil %s de test", space);
  if (!cmsMLUsetASCII(value, "en", "US", english) || !cmsMLUsetASCII(value, "fr", "FR", french)) {
    fail("could not write RGB profile labels");
  }
  return value;
}

/* P3 primaries with gamma 2.2, NOT the standard Display-P3 transfer curve. */
static cmsHPROFILE create_p3_gamma22(void) {
  cmsCIExyY white = {0.3127, 0.3290, 1.0};
  cmsCIExyYTRIPLE primaries = {{0.6800, 0.3200, 1.0}, {0.2650, 0.6900, 1.0}, {0.1500, 0.0600, 1.0}};
  cmsToneCurve *curve = cmsBuildGamma(NULL, 2.2);
  cmsToneCurve *curves[3] = {curve, curve, curve};
  cmsHPROFILE profile;
  if (curve == NULL) {
    fail("could not create gamma-2.2 transfer curve");
  }
  profile = cmsCreateRGBProfile(&white, &primaries, curves);
  cmsFreeToneCurve(curve);
  if (profile == NULL) {
    fail("could not create P3-primary gamma-2.2 profile");
  }
  return profile;
}

int main(int argc, char **argv) {
  cmsHPROFILE profile;
  cmsMLU *description;
  cmsMLU *copyright;
  int version;

  if (argc != 4 || (strcmp(argv[1], "srgb") != 0 && strcmp(argv[1], "display-p3") != 0)) {
    fail("usage: make_rgb_icc_fixture {srgb|display-p3} OUTPUT {2|4}");
  }
  version = strcmp(argv[3], "2") == 0 ? 2 : 4;
  profile = strcmp(argv[1], "srgb") == 0 ? cmsCreate_sRGBProfile() : create_p3_gamma22();
  if (profile == NULL) {
    fail("could not create RGB profile");
  }
  cmsSetProfileVersion(profile, (cmsFloat64Number)version);
  description = labels(strcmp(argv[1], "srgb") == 0 ? "sRGB" : "P3-primary gamma-2.2");
  copyright = labels(strcmp(argv[1], "srgb") == 0 ? "sRGB" : "P3-primary gamma-2.2");
  if (!cmsWriteTag(profile, cmsSigProfileDescriptionTag, description) ||
      !cmsWriteTag(profile, cmsSigCopyrightTag, copyright) || !cmsSaveProfileToFile(profile, argv[2])) {
    fail("could not save RGB profile");
  }
  cmsMLUfree(description);
  cmsMLUfree(copyright);
  cmsCloseProfile(profile);
  if (version == 2) {
    normalize_owned_v2_description_extent(argv[2]);
  }
  stamp_header(argv[2]);
  return 0;
}
