#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef struct Buffer {
  uint32_t len;
  uint8_t *data;
} Buffer;

struct Buffer get_turbo_data_dir(void);

struct Buffer changed_files(struct Buffer buffer);

struct Buffer previous_content(struct Buffer buffer);
