/* TEST-SIDE ONLY: real libheif raw/display consumer. Never sanitizer code. */
#include <libheif/heif.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void check(struct heif_error error) {
  if (error.code != heif_error_Ok) {
    fprintf(stderr, "libheif consumer error %d/%d: %s\n", error.code,
            error.subcode, error.message);
    exit(1);
  }
}

int main(int argc, char **argv) {
  if (argc != 4 || (strcmp(argv[2], "raw") && strcmp(argv[2], "display"))) return 1;
  struct heif_context *context = heif_context_alloc();
  if (!context) return 1;
  check(heif_context_read_from_file(context, argv[1], NULL));
  if (heif_context_get_number_of_top_level_images(context) != 1) return 1;
  struct heif_image_handle *handle = NULL;
  check(heif_context_get_primary_image_handle(context, &handle));
  struct heif_decoding_options *options = heif_decoding_options_alloc();
  if (!options) return 1;
  /* Test two distinct rendering views; leave other library defaults untouched. */
  options->ignore_transformations = argv[2][0] == 'r';
  options->strict_decoding = 1;
  struct heif_image *image = NULL;
  check(heif_decode_image(handle, &image, heif_colorspace_RGB,
                         heif_chroma_interleaved_RGBA, options));
  int warnings = heif_image_get_decoding_warnings(image, 0, NULL, 0);
  if (warnings) {
    for (int i = 0; i < warnings; ++i) {
      struct heif_error warning;
      heif_image_get_decoding_warnings(image, i, &warning, 1);
      fprintf(stderr, "libheif consumer warning: %s\n", warning.message);
    }
    return 1;
  }
  int width = heif_image_get_width(image, heif_channel_interleaved);
  int height = heif_image_get_height(image, heif_channel_interleaved);
  size_t stride = 0;
  const uint8_t *plane = heif_image_get_plane_readonly2(image, heif_channel_interleaved, &stride);
  if (!plane || width <= 0 || height <= 0 || width > 64 || height > 64 || stride < (size_t)width * 4) return 1;
  FILE *file = fopen(argv[3], "wb");
  if (!file) return 1;
  for (int y = 0; y < height; ++y) {
    if (fwrite(plane + (size_t)y * stride, 4, (size_t)width, file) != (size_t)width) return 1;
  }
  if (fclose(file)) return 1;
  printf("{\"width\":%d,\"height\":%d,\"warnings\":0,\"has_alpha\":%d,\"libheif\":\"%s\"}\n", width, height,
         heif_image_handle_has_alpha_channel(handle), heif_get_version());
  heif_image_release(image);
  heif_decoding_options_free(options);
  heif_image_handle_release(handle);
  heif_context_free(context);
  return 0;
}
