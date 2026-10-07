/* Generates owned, synthetic ICC fixtures with pinned LittleCMS. */
#include <lcms2.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void fail(const char *message) {
  fputs(message, stderr);
  fputc('\n', stderr);
  exit(1);
}

static void stamp_header(const char *path) {
  FILE *file = fopen(path, "r+b");
  unsigned char date[12] = {0, 0, 7, 234, 0, 10, 0, 7, 0, 12, 0, 34};
  if (file == NULL) {
    fail("could not reopen synthetic profile");
  }
  if (fseek(file, 24, SEEK_SET) != 0 || fwrite(date, 1, sizeof(date), file) != sizeof(date) ||
      fseek(file, 48, SEEK_SET) != 0 || fwrite("JAUDTEST", 1, 8, file) != 8 ||
      fseek(file, 80, SEEK_SET) != 0 || fwrite("OWND", 1, 4, file) != 4 || fclose(file) != 0) {
    fail("could not stamp synthetic profile header");
  }
}

static cmsMLU *labels(void) {
  cmsMLU *value = cmsMLUalloc(NULL, 2);
  if (value == NULL || !cmsMLUsetASCII(value, "en", "US", "Owned Device Link") ||
      !cmsMLUsetASCII(value, "fr", "FR", "Profil de liaison propriétaire")) {
    fail("could not allocate profile labels");
  }
  return value;
}

int main(int argc, char **argv) {
  cmsHPROFILE profile;
  cmsMLU *description;
  cmsMLU *copyright;
  cmsUInt32Number version;

  if (argc != 4) {
    fail("usage: make_icc_fixture INPUT OUTPUT VERSION");
  }
  profile = cmsOpenProfileFromFile(argv[1], "r");
  if (profile == NULL) {
    fail("could not open generated device-link profile");
  }
  version = strcmp(argv[3], "2") == 0 ? 2 : 4;
  cmsSetProfileVersion(profile, (cmsFloat64Number)version);
  description = labels();
  copyright = labels();
  if (!cmsWriteTag(profile, cmsSigProfileDescriptionTag, description) ||
      !cmsWriteTag(profile, cmsSigCopyrightTag, copyright)) {
    fail("could not write profile labels");
  }
  cmsMLUfree(description);
  cmsMLUfree(copyright);
  if (!cmsSaveProfileToFile(profile, argv[2])) {
    fail("could not save synthetic profile");
  }
  cmsCloseProfile(profile);
  stamp_header(argv[2]);
  return 0;
}
