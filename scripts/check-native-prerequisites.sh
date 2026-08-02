#!/usr/bin/env bash
set -euo pipefail

failures=()

if ! command -v clang >/dev/null 2>&1; then
  failures+=("clang is not available")
else
  resource_dir=$(clang -print-resource-dir 2>/dev/null || true)
  if [[ -z "$resource_dir" || ! -r "$resource_dir/include/stdbool.h" ]]; then
    failures+=("clang resource headers are incomplete")
  elif ! printf '#include <stdbool.h>\n_Bool aozora_bindgen_probe;\n' \
    | clang -fsyntax-only -x c - >/dev/null 2>&1; then
    failures+=("clang cannot preprocess the standard C headers required by bindgen")
  fi
fi

shopt -s nullglob
libclang_headers=(/usr/include/clang-c/Index.h /usr/lib/llvm-*/include/clang-c/Index.h)
if [[ -n "${LIBCLANG_PATH:-}" ]]; then
  libclang_headers+=("$LIBCLANG_PATH/../include/clang-c/Index.h")
fi
libclang_header_ready=false
for header in "${libclang_headers[@]}"; do
  if [[ -r "$header" ]]; then
    libclang_header_ready=true
    break
  fi
done
if [[ "$libclang_header_ready" != true ]]; then
  failures+=("libclang development headers are not available")
fi

if [[ -n "${LIBCLANG_PATH:-}" ]]; then
  libclang_libraries=("$LIBCLANG_PATH"/libclang.so*)
  libclang_library_ready=false
  for library in "${libclang_libraries[@]}"; do
    if [[ -r "$library" ]]; then
      libclang_library_ready=true
      break
    fi
  done
  if [[ "$libclang_library_ready" != true ]]; then
    failures+=("LIBCLANG_PATH does not contain libclang")
  fi
elif ! ldconfig -p 2>/dev/null | grep -Eq 'libclang(-[0-9]+)?\.so'; then
  failures+=("libclang is not available to the dynamic linker")
fi

if ! command -v valgrind >/dev/null 2>&1; then
  failures+=("valgrind is not available")
elif ! valgrind --tool=callgrind --help >/dev/null 2>&1; then
  failures+=("valgrind cannot start the Callgrind tool")
fi

if ((${#failures[@]} > 0)); then
  printf 'Native prerequisites are not ready:\n' >&2
  printf '  - %s\n' "${failures[@]}" >&2
  printf '%s\n' \
    'Install Clang, libclang development headers, and Valgrind with the host package manager.' \
    'Debian/Ubuntu: sudo apt-get install clang libclang-dev valgrind' >&2
  exit 1
fi
