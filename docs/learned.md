# Aprendizajes (learned log)

> **Propósito del archivo**: bitácora acumulativa de aprendizajes técnicos del
> proyecto, con formato estable para compilar entre repos — los repos sucesivos
> que se creen con este objetivo llevan su `learned.md` y el compilado los
> recorre leyendo los encabezados uniformes. Cada entrada: **lo que pasó**,
> **la regla** que evita repetirlo, y **la evidencia** (run/commit/dato).
>
> Convención de entrada: `## [topic] — título` + `**Fecha**: ...` + cuerpo con
> `Regla:` y `Evidencia:`. Temas estables (palabras de compuesto): `scaleway`,
> `supabase`, `ci`, `rust`, `operacion`.

---

## [scaleway] scaleway_function no despliega sin `deploy = true`

**Fecha**: 2026-10-22/23 (v0.1.2–v0.1.5)

El atributo `deploy` de `scaleway_function` (provider `scaleway`) defaultea a
`false`: el `apply` sube el zip y actualiza la configuración, pero el **build y
el swap solo ocurren con el Restart manual del dashboard**. Con `deploy = true`
Terraform también lanza el deploy y espera el build.

- **Regla**: en todo proyecto Scaleway Functions + Terraform, declarar
  `deploy = true` desde el primer día. El apply tardará el build — es el costo
  correcto, no un cuelgue.
- **Evidencia**: deploys v0.1.2..v0.1.4 con el paso manual fantasma; el run 13
  (v0.1.5) midió Apply en **7 min** con el `deploy = true`.

## [scaleway] El build frío de Rust tarda >10 min — sondear el estado del build en el CI falla y cobra

**Fecha**: 2026-10-22/23

El sondeo del smoke contra el estado del build (40×15s) se agotó en v0.1.2 y
v0.1.3 con deploys correctos — firma del run de ~12 min exactos; subir a 80
intentos solo lo demoró más (v0.1.4 pagó 20 min). Con `deploy = true` el sondeo
quedó sobrante: el apply ya espera.

- **Regla**: **nunca sondear el estado del build en el CI**. La espera del
  provider (`deploy = true`) corre dentro del apply que ya se paga; la espera
  en el CI **cobra dos veces**: runner-minutes facturables E invocaciones
  facturables de la función (cada poll de la API y cada golpe de `/health`),
  amplificado cada vez que el job se re-corré. El chequeo post-deploy queda
  como una sola compuerta HTTP con margen de SEGUNDOS.
- **Evidencia**: run 10 (12 min), run 11 (12 min), run 12 (20.5 min); el run 14
  (v0.1.6): Apply 11.9 min + **Smoke de 12 segundos** — run verde completo.
- **Costo medido del smoke sobrande (v0.1.5)**: el run 12 gastó ~20.5 min de
  runner + ~86 invocaciones de producción en sondear un build correcto.

## [scaleway] Logs y métricas del runtime rust196 vs AWS Lambda

**Fecha**: 2026-10-22/23 — comparación del operador

Quejas concretas del operador contra Scaleway Functions, siendo AWS Lambda el
referente:

1. **El log del evento no va acompañado de su costo**: la línea del wrapper
   (`Function triggered: /path`) no incluye cuánto tiempo tomó el evento ni
   cuántos recursos consumió. En Lambda, cada invocación loggea `REPORT` con
   request-id, **duration**, **billed duration**, **memory used** — todo en la
   misma corriente.
2. **Las métricas van por aparte**: la interfaz de métricas (Cockpit) es una
   consola distinta de la de logs — pasos adicionales vs Lambda (donde
   CloudWatch mezcla logs y métricas del request en un mismo lugar).
3. **Las métricas tampoco miden el tiempo de ejecución del evento**: solo
   muestran **CPU y memoria** — para correlacionar latencia hay que inferirla
   (o cronometrar por fuera).
4. **Primera medición real de la función** (configuración base actual, min_scale 0 /
   max_scale 1 / 128 MiB): el evento típico consumió **~48 MiB de memoria y
   ~20% de CPU** — la respuesta del `/health` (0.5-0.7 s de latencia medida
   desde fuera, en gran parte cold-start del contenedor).

- **Regla**: la app debe auto-instrumentar lo que la plataforma no da: la
  capa `TraceLayer` (latencia por request), los eventos D18 con `ts` propio y
  el log de arranque de BD (PR-1) son el sustituto del `REPORT` de Lambda.
  Presupuesto la memoria del crate con la medición (48 MiB / 128 MiB deja
  ~3× de margen — vigilado cuando crezcan las rutas).
- **Evidencia**: captura de Functions Logs con solo líneas del wrapper; la
  medición de 48 MiB / 20% CPU en el panel del operador (2026-10-22).

## [scaleway] La instalación de `try_init` jamás se traga en silencio (y los logs de la app funcionan — era la latencia del shipper)

**Fecha**: 2026-10-22/23

"No hay logs de la app" resultó ser **latencia de sincronización del
shipper de logs de la plataforma**: el registro con timestamp propio aparecía
minutos tarde en la vista. El subscriber de tracing funciona (JSON en stdout
del runtime legacy y stderr del fix v0.1.4 — ambos canales capturados; el
revert de v0.1.6 lo devolvió a stdout). El wrapper NO bloquea al subscriber
propio de la app.

- **Regla**: ante un "no hay logs", **el timestamp del log es la verdad, no la
  hora de vista** — esperar la ingesta (minutos) y compilar por
  instancia/deployment antes de atribuirlo. Y la regla Rust transversal: el
  fallo de `try_init` del subscriber NUNCA queda en `let _ =` — eprintln con
  el motivo (una sola vez lo detecta: o el conflicto del subscriber, o la
  salida del canal).
- **Evidencia**: la línea JSON `tower_http::trace::on_failure` ERROR con
  timestamp `16:24:31Z` proveniente del build viejo; el log de arranque de BD
  (`latency_ms: 161`) visible tras el sync (2026-10-23).

## [scaleway] Ventana 5xx propia del apply-secrets→build-nuevo

**Fecha**: 2026-10-22

El apply intercambia secrets/env ANTES de que el build nuevo esté listo: el
build viejo sigue sirviendo con la config que ya cambió → 500 en cada
cold-start hasta el swap (el v0.1.3: el build viejo perdió `SUPABASE_JWT_SECRET`
y respondió 500 por toda la ventana).

- **Regla**: durante un deploy, un 5xx "de configuración" en producción puede
  ser DEL BUILD VIEJO — correlacionar deployment-id antes de culpar al código
  nuevo (la ventana termina cuando el swap llega, con `deploy = true` el apply
  la espera).
- **Evidencia**: la ventana de 500 de v0.1.3 (16:39–16:55 aprox, deployment-00008).

## [supabase] JWKS/ES256: nueva era de claves asimétricas (post-octubre-2025)

**Fecha**: 2026-10-22 (T006-amend, v0.1.3)

Los proyectos Supabase nuevos firman con **claves asimétricas (ES256)**: el
legacy `SUPABASE_JWT_SECRET` ya no existe — la credencial del proyecto es el
**JWKS URL** (público). El verificador (`JwksVerifier`) resuelve la clave por
`kid`, cachea el set y re-fetchea ante un kid desconocido (rotación sin
re-deploy); un fallo de fetch es 503 (dependencia), nunca 401.

- **Regla**: la firma de credenciales de Supabase nunca se asume HS256; el
  JWKS endpoint del proyecto `GET /auth/v1/.well-known/jwks.json` es la
  primera prueba del proyecto. Y en local: el goTrue legacy **no emite
  `issuer`** y su JWKS sale con `keys: []` (no publica claves simétricas) —
  los tests de flujos protegidos en local usan el verificador HS256 del
  `AppState::for_tests()`, el CI el JWKS real del cloud.
- **Evidencia**: el JWKS de produccion: 1 clave EC P-256/ES256 validada en
  curl (v0.1.3, 2026-10-23).

## [supabase] Pooler modo sesión vs transacción en serverless

**Fecha**: 2026-10-22/23

La red sin IPv6 no conecta directa a Supabase: el pooler es la única vía —
`aws-1-eu-central-1.pooler.supabase.com`. Puerto **5432 = modo sesión** (cada
conexión del pool mantiene una sesión pegada — compite contra el techo de
conexiones del proyecto), puerto **6543 = modo transacción** (libera por
transacción — el correcto para una función con pool interno que abre/cierra).

- **Regla**: migraciones (CLI, una conexión a la vez) → 5432; **la API →
  6543** (secret de producción, cambio de una línea — pendiente de ejecutar al
  cierre de la fase US1). La escala de las funciones NUNCA debe competir contra
  el límite de conexiones: con instancias que abren/cierran, el modo sesión
  muerde el techo.
- **Evidencia**: el `.env` del operador (5432 en curso) + el comentario propio
  señalando 6543 para la API; TCP probe OK.

## [ci] La cobertura se mide ANTES del push y el camino async se cubre en el primer PR

**Fecha**: 2026-10-22 (ratchet 92.79/93.66 y T012 en 0/105)

Dos fallos de cobertura, dos reglas:

1. **T012 inicial**: el handler nuevo quedó **0/105 cubierto** porque su
   corredor dependía de una tarea posterior (T011) — el CI ratchet falla. El
   arreglo: unit tests del handler contra un fake in-process (axum — es
   dependencia de prod; sin dev-deps de mock http) y las pruebas que caben
   **ahora**.
2. El ratchet del PR del JWKS (92.79% vs base 93.66%): los tests cubrían solo
   el núcleo fácil; el wiring de producción usa otra implementación que la del
   estado de test — lo no cubierto es la frontera entre ambos.

- **Regla del operador**: el checklist pre-push es fmt → clippy → tests →
  **`cargo llvm-cov` con las exclusiones del CI** — el push no es el momento
  de descubrir cobertura. Y si una porción depende de una tarea posterior: se
  declara en el PR, no se descubre en el CI.
- **Evidencia**: handlers.rs 0%→98.4% tras las pruebas con el fake (la delta
  recuperó el ratchet con margen: +206 líneas cubiertas).

## [ci] Los tests de integración corrieron muertos: el orden del merge de dotenv

**Fecha**: 2026-10-23 (harness T010)

`load_env_files` cargaba `.env` primero con set-if-absent: el
`DATABASE_URL` **cloud** del `.env` ganaba y la guardia de localidad
fallaba — **todos los tests de integración se salteaban en silencio**, aquí y
en el CI (que además exporta `SUPABASE_SERVICE_ROLE_KEY`, no `SERVICE_ROLE_KEY`).
Toda la capa de integración (T010/T011+) "pasaba" sin correr.

- **Regla**: el merge de dotenv resuelve **por última escritura** (el stack de
  test gana) y el proceso conserva la última palabra; los alias (`SERVICE_ROLE_KEY`
  ← `SUPABASE_SERVICE_ROLE_KEY`) viven en el harness, no en la config de la app.
  Un test que se saltea silenciosamente NO es un test verde — el resultado
  "2 passed en 0.00s" era la trampa.
- **Evidencia**: la guardia corriendo de verdad contra el stack `pap` local:
  2/2 integration ok en 0.05s (2026-10-23).

## [rust/axum] El deny-by-default vive en el EXTRACTOR, no en el router

**Fecha**: 2026-10-23

En axum, un handler sin el extractor de autenticación es **anónimo aunque su
ruta viva en un router anidado "protegido"**: `/v1/cv/versions/{id}` extraía
solo `Path` y un anónimo pasaba a 503 donde FR-011 exige 401. El wiring test
lo delató.

- **Regla**: todo handler protegido lleva `AuthenticatedUser` como PRIMER
  extractor — ninguna convención de router protege nada. La prueba del wiring
  (cada ruta protegida anónima → 401) queda permanente.
- **Evidencia**: `wiring::the_protected_routes_require_a_credential` fallando
  por la ruta de versión; arreglado con el extractor en `stub_version_503`.

## [operacion] Las estabilizaciones van en las PRIMERAS tareas, no en la cola de polish

**Fecha**: 2026-10-23 — regla del operador

Las tareas de operación/observabilidad (prueba de DB en el arranque, deploy
automatizado, smoke sin sondeo, logging verificado) **pertenecen a las
primeras tareas del plan** — su valor y prioridad se cobra antes — para que el
desarrollo de las tareas complejas sea más preciso.

- **Regla**: al armar el orden de tareas de cualquier feature/proyecto:
  estabilidad/observabilidad adelantada. Y los hallazgos de plataforma que
  aparezcan a mitad del ciclo se absorben como tareas tempranas del **ciclo en
  curso** — nunca a una cola final de "polish".
- **Evidencia**: esta sesión — las "tareas pequeñas" (deploy=true, smoke slim,
  log de arranque, diagnóstico del shipper) dieron más valor operativo que
  cualquier handler; el ciclo US1/T012 arrancó sobre una plataforma estabilizada.

## [operacion] Los PRs se apilan y la cobertura viaja con su feature

**Fecha**: 2026-10-22/23

Flujo que este proyecto ya valida: rama por tarea, ≤6 archivos, squash/merge
con revisión manual entre PRs; el fix apilado se rebase sobre la rama que
reportó (el rustsec rebaseado sobre la rama del PR-1 que reportó el aviso);
la cobertura del handler nuevo viaja en el MISMO PR que la implementa.

- **Regla**: cuando un hallazgo surge sobre UNA rama, el fix se rebase sobre
  ella (la pila lineal); nunca desde main — la revisión queda como diff
  incremental encima de lo ya sabido.
- **Evidencia**: rebase `fix/rustsec-2026-0385` sobre `feature/T012-startup-db-ping`
  tras el feedback del operador.