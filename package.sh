#!/usr/bin/env bash
# Empaqueta el zip de fuentes que la plataforma Scaleway compila (plan §12.2).
# Terraform despliega este zip (`zip_file` + `zip_hash`); cambiar el contenido
# gatilla el re-deploy de la función.
#
# Mismos patterns que tenía el serverless.yml: Cargo.toml, Cargo.lock,
# src/** y migrations/** — excluye src/main.rs (servidor de desarrollo local).
set -euo pipefail
cd "$(dirname "$0")/.."

rm -f infra/function.zip

zip --quiet --recurse-paths \
  infra/function.zip \
  Cargo.toml Cargo.lock src \
  -x 'src/main.rs'

if [ -d migrations ]; then
  zip --quiet --recurse-paths infra/function.zip migrations
fi

echo "function.zip: $(sha256sum infra/function.zip | cut -d' ' -f1)"