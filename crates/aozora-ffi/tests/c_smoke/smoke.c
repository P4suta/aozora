
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stddef.h>

#include "aozora.h"

static int check(const char *what, int condition) {
    if (!condition) {
        fprintf(stderr, "FAIL: %s\n", what);
        return 1;
    }
    return 0;
}

int main(void) {
    int failures = 0;
    char empty_envelope[64];
    int empty_envelope_len =
        snprintf(empty_envelope, sizeof(empty_envelope),
                 "{\"schemaVersion\":%u,\"data\":[]}", AOZORA_SCHEMA_VERSION);
    failures += check("wire expectation fits buffer",
                      empty_envelope_len > 0 &&
                          (size_t)empty_envelope_len < sizeof(empty_envelope));

    const char *src = "Hello, world.";
    AozoraDocument *doc = NULL;
    int32_t status = aozora_document_new((const uint8_t *)src, strlen(src), &doc);
    failures += check("aozora_document_new returns Ok", status == 0);
    failures += check("aozora_document_new produces non-null handle", doc != NULL);

    AozoraBytes html = {NULL, 0, 0};
    status = aozora_document_to_html(doc, &html);
    failures += check("aozora_document_to_html returns Ok", status == 0);
    failures += check("html buffer is non-empty", html.len > 0);

    char *html_str = (char *)malloc(html.len + 1);
    memcpy(html_str, html.ptr, html.len);
    html_str[html.len] = '\0';
    failures += check("html contains \"Hello\"", strstr(html_str, "Hello") != NULL);
    free(html_str);
    aozora_bytes_free(html);

    AozoraBytes diag = {NULL, 0, 0};
    status = aozora_document_diagnostics_json(doc, &diag);
    failures += check("aozora_document_diagnostics_json returns Ok", status == 0);
    failures += check("diagnostics JSON is the empty envelope",
                      diag.len == (size_t)empty_envelope_len &&
                          memcmp(diag.ptr, empty_envelope,
                                 (size_t)empty_envelope_len) == 0);
    aozora_bytes_free(diag);

    AozoraBytes source = {NULL, 0, 0};
    status = aozora_document_to_source(doc, &source);
    failures += check("aozora_document_to_source returns Ok", status == 0);
    failures += check("source buffer is non-empty", source.len > 0);
    char *source_str = (char *)malloc(source.len + 1);
    memcpy(source_str, source.ptr, source.len);
    source_str[source.len] = '\0';
    failures += check("round-trip source contains \"Hello\"",
                      strstr(source_str, "Hello") != NULL);
    free(source_str);
    aozora_bytes_free(source);

    AozoraBytes cpairs = {NULL, 0, 0};
    status = aozora_document_container_pairs_json(doc, &cpairs);
    failures += check("aozora_document_container_pairs_json returns Ok", status == 0);
    failures += check("container-pairs JSON is the empty envelope",
                      cpairs.len == (size_t)empty_envelope_len &&
                          memcmp(cpairs.ptr, empty_envelope,
                                 (size_t)empty_envelope_len) == 0);
    aozora_bytes_free(cpairs);

    AozoraBytes gaiji = {NULL, 0, 0};
    status = aozora_document_gaiji_json(doc, &gaiji);
    failures += check("aozora_document_gaiji_json returns Ok", status == 0);
    failures += check("gaiji JSON is the empty envelope",
                      gaiji.len == (size_t)empty_envelope_len &&
                          memcmp(gaiji.ptr, empty_envelope,
                                 (size_t)empty_envelope_len) == 0);
    aozora_bytes_free(gaiji);

    size_t src_len = 0;
    status = aozora_document_source_byte_len(doc, &src_len);
    failures += check("aozora_document_source_byte_len returns Ok", status == 0);
    failures += check("source byte length equals input length",
                      src_len == strlen(src));

    AozoraBytes slugs = {NULL, 0, 0};
    status = aozora_slugs_json(&slugs);
    failures += check("aozora_slugs_json returns Ok", status == 0);
    failures += check("slug catalogue JSON is non-empty", slugs.len > 0);
    {
        char *slugs_str = (char *)malloc(slugs.len + 1);
        memcpy(slugs_str, slugs.ptr, slugs.len);
        slugs_str[slugs.len] = '\0';
        failures += check("slug catalogue carries the wire envelope",
                          strstr(slugs_str, "schemaVersion") != NULL &&
                          strstr(slugs_str, "data") != NULL);
        free(slugs_str);
    }
    aozora_bytes_free(slugs);

    aozora_document_free(doc);

    AozoraDocument *bad = NULL;
    status = aozora_document_new(NULL, 0, &bad);
    failures += check("null src returns NullInput status", status == -1);
    failures += check("null src leaves out_doc null", bad == NULL);

    const char *pua = "abc\xEE\x80\x81 def";
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

    AozoraBytes pua_text = {NULL, 0, 0};
    status = aozora_document_diagnostics_text(pua_doc, &pua_text);
    failures += check("aozora_document_diagnostics_text returns Ok", status == 0);
    failures += check("plain-text diagnostics are non-empty", pua_text.len > 0);
    char *pua_text_str = (char *)malloc(pua_text.len + 1);
    memcpy(pua_text_str, pua_text.ptr, pua_text.len);
    pua_text_str[pua_text.len] = '\0';
    failures += check("plain-text diagnostics carry the aozora:: code",
                      strstr(pua_text_str, "aozora::") != NULL);
    free(pua_text_str);
    aozora_bytes_free(pua_text);
    aozora_document_free(pua_doc);

#if SIZE_MAX > 0xFFFFFFFFu
    {
        uint8_t sentinel = 0;
        const uint8_t *dangling = &sentinel;
        size_t oversize = (size_t)0xFFFFFFFFu + 1u;
        AozoraDocument *huge = NULL;
        status = aozora_document_new(dangling, oversize, &huge);
        failures += check("oversize src_len returns SourceTooLarge", status == -5);
        failures += check("oversize src_len leaves out_doc null", huge == NULL);
    }
#endif

    if (failures == 0) {
        printf("c_smoke: all checks passed\n");
        return 0;
    }
    fprintf(stderr, "c_smoke: %d check(s) failed\n", failures);
    return 1;
}
