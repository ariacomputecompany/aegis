#ifndef AEGIS_HOST_ABI_H
#define AEGIS_HOST_ABI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum AegisHostStatus {
  AEGIS_HOST_OK = 0,
  AEGIS_HOST_ERROR = 1
} AegisHostStatus;

typedef struct AegisHostBuffer {
  uint8_t* ptr;
  size_t len;
} AegisHostBuffer;

typedef void* AegisHostHandle;

typedef AegisHostStatus (*AegisHostApi)(
    AegisHostHandle ctx,
    const uint8_t* input_ptr,
    size_t input_len,
    AegisHostBuffer* output);

typedef void (*AegisHostFree)(AegisHostHandle ctx, AegisHostBuffer buffer);

typedef struct AegisHostFunctionTable {
  AegisHostApi install_runtime;
  AegisHostApi eval_js;
  AegisHostApi send_batch;
  AegisHostApi snapshot_dom;
  AegisHostApi inject_session;
  AegisHostApi snapshot_session;
  AegisHostApi drain_events;
  AegisHostApi navigate;
  AegisHostApi pump;
  AegisHostApi snapshot_chrome_state;
  AegisHostApi go_back;
  AegisHostApi go_forward;
  AegisHostApi reload_page;
  AegisHostApi stop_load;
  AegisHostFree free_buffer;
} AegisHostFunctionTable;

AegisHostHandle aegis_create_host(const uint8_t* input_ptr, size_t input_len);
const char* aegis_last_error_message(void);
void aegis_destroy_host(AegisHostHandle handle);
AegisHostFunctionTable aegis_get_function_table(void);

#ifdef __cplusplus
}
#endif

#endif
