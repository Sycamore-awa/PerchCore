#ifndef PERCH_CORE_H
#define PERCH_CORE_H

#include <stddef.h>
#include <stdint.h>

/* Existing symbols and ownership rules remain ABI v1; new symbols are additive. */
#define PERCH_CORE_ABI_VERSION 1

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Invoke a method with a UTF-8 JSON object. Lengths are in bytes.
 * Input buffers are borrowed only for the duration of this synchronous call.
 * Non-null input pointers must address at least their specified byte lengths.
 * A NULL pointer or zero length is treated as empty input. Empty/whitespace args
 * means {}. Invalid UTF-8 produces FFI_BAD_METHOD_PTR / FFI_BAD_ARGS_PTR.
 * The returned NUL-terminated UTF-8 JSON string is allocated by Rust.
 * Free it exactly once with perch_string_free(), never with free().
 * Business errors are returned as {"error":{"code":"...","message":"..."}}.
 * Treat a NULL result as a failed call.
 * Stateful methods use the non-destroyable legacy context. Handles are integer
 * IDs, including 0, shared across book/txt/pipeline kinds. Current quotas are
 * 8 book, 4 txt, 1 pipeline for this context. Coordinate use and close.
 * Duplicate close returns closed:false; wrong-kind close does not free anything.
 * Character offsets are Unicode scalars; txtByteOffset uses decoded UTF-8 bytes.
 * API v1.4 book.locate returns decoded TXT charOffset/UTF-8 byteOffset or EPUB
 * href/nodePath with separate scalarOffset/utf16Offset fields at each endpoint.
 * Source coordinates refer to the source version used when indexing the book.
 * See docs/ffi.md, docs/api-reference.md and api/ for all 35 method contracts.
 * Schema wrappers {method,args}/{method,result} are offline tooling only:
 * pass args itself here. Runtime does not automatically enforce those schemas.
 */
char *perch_call(const uint8_t *method_ptr, size_t method_len,
                 const uint8_t *args_ptr, size_t args_len);

/*
 * Invoke a method inside an explicit context returned by context.open.
 * Available in JSON API v1.2 and later. Method/args/result memory rules are
 * identical to perch_call(). Existing business JSON shapes are unchanged.
 * Context 0, an unknown context, or a closed context returns
 * CONTEXT_BAD_HANDLE. Handles belong to their creating context; cross-context
 * use returns CONTEXT_HANDLE_MISMATCH. context.open/context.close themselves
 * must be invoked with perch_call(). Each context has independent fixed quotas.
 */
char *perch_context_call(uint64_t context,
                         const uint8_t *method_ptr, size_t method_len,
                         const uint8_t *args_ptr, size_t args_len);

/* Accepts NULL. Otherwise ptr must be an unfreed result of either call entry. */
void perch_string_free(char *ptr);

#ifdef __cplusplus
}
#endif

#endif
