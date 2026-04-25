/*
 * End-to-end C ABI smoke test for aozora-ffi.
 *
 * This program is the reference C consumer of the public surface
 * exposed by `crates/aozora-ffi/src/lib.rs`. It exercises every
 * `aozora_*` function on the happy path:
 *
 *   1. aozora_document_new            — construct from bytes
 *   2. aozora_document_to_html        — render HTML
 *   3. aozora_document_diagnostics_json — diagnostic projection
 *   4. aozora_bytes_free              — release returned buffers
 *   5. aozora_document_free           — release the document handle
 *
 * Build / run is driven by `tests/c_smoke/run.sh` (which builds the
 * cdylib first, then compiles and runs this program). Returns 0 on
 * success, non-zero on any unexpected status code or content.
 *
 * Header generation is deferred to the future cbindgen integration;
 * for now the prototypes are declared inline below so the smoke
 * runs without any external tool.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stddef.h>

/* --- Inline prototypes mirroring crates/aozora-ffi/src/lib.rs --- */

typedef struct AozoraDocument AozoraDocument;

typedef struct {
    uint8_t *ptr;
    size_t len;
    size_t cap;
} AozoraBytes;

extern int32_t aozora_document_new(const uint8_t *src_ptr, size_t src_len,
                                    AozoraDocument **out_doc);
extern int32_t aozora_document_to_html(const AozoraDocument *doc,
                                        AozoraBytes *out_html);
extern int32_t aozora_document_diagnostics_json(const AozoraDocument *doc,
                                                 AozoraBytes *out_json);
extern void aozora_document_free(AozoraDocument *doc);
extern void aozora_bytes_free(AozoraBytes bytes);

/* --- Test --- */

static int check(const char *what, int condition) {
    if (!condition) {
        fprintf(stderr, "FAIL: %s\n", what);
        return 1;
    }
    return 0;
}

int main(void) {
    int failures = 0;

    /* 1. parse "Hello, world." */
    const char *src = "Hello, world.";
    AozoraDocument *doc = NULL;
    int32_t status = aozora_document_new((const uint8_t *)src, strlen(src), &doc);
    failures += check("aozora_document_new returns Ok", status == 0);
    failures += check("aozora_document_new produces non-null handle", doc != NULL);

    /* 2. render HTML */
    AozoraBytes html = {NULL, 0, 0};
    status = aozora_document_to_html(doc, &html);
    failures += check("aozora_document_to_html returns Ok", status == 0);
    failures += check("html buffer is non-empty", html.len > 0);

    /* HTML output must contain "Hello" somewhere. We don't pin the
     * exact serialisation — the renderer can wrap things in tags as
     * it pleases. */
    char *html_str = (char *)malloc(html.len + 1);
    memcpy(html_str, html.ptr, html.len);
    html_str[html.len] = '\0';
    failures += check("html contains \"Hello\"", strstr(html_str, "Hello") != NULL);
    free(html_str);
    aozora_bytes_free(html);

    /* 3. diagnostics — empty JSON array for clean input */
    AozoraBytes diag = {NULL, 0, 0};
    status = aozora_document_diagnostics_json(doc, &diag);
    failures += check("aozora_document_diagnostics_json returns Ok", status == 0);
    failures += check("diagnostics JSON is exactly \"[]\"",
                      diag.len == 2 && diag.ptr[0] == '[' && diag.ptr[1] == ']');
    aozora_bytes_free(diag);

    /* 4. free the document */
    aozora_document_free(doc);

    /* 5. null input — must return NullInput status */
    AozoraDocument *bad = NULL;
    status = aozora_document_new(NULL, 0, &bad);
    failures += check("null src returns NullInput status", status == -1);
    failures += check("null src leaves out_doc null", bad == NULL);

    /* 6. PUA collision — must yield a diagnostic */
    const char *pua = "abc\xEE\x80\x81 def"; /* U+E001 in UTF-8 = EE 80 81 */
    AozoraDocument *pua_doc = NULL;
    status = aozora_document_new((const uint8_t *)pua, strlen(pua), &pua_doc);
    failures += check("PUA-source aozora_document_new Ok", status == 0);
    AozoraBytes pua_diag = {NULL, 0, 0};
    status = aozora_document_diagnostics_json(pua_doc, &pua_diag);
    failures += check("PUA diagnostics call Ok", status == 0);
    char *pua_diag_str = (char *)malloc(pua_diag.len + 1);
    memcpy(pua_diag_str, pua_diag.ptr, pua_diag.len);
    pua_diag_str[pua_diag.len] = '\0';
    failures += check("PUA diagnostics mention source_contains_pua",
                      strstr(pua_diag_str, "source_contains_pua") != NULL);
    free(pua_diag_str);
    aozora_bytes_free(pua_diag);
    aozora_document_free(pua_doc);

    if (failures == 0) {
        printf("c_smoke: all checks passed\n");
        return 0;
    }
    fprintf(stderr, "c_smoke: %d check(s) failed\n", failures);
    return 1;
}
