#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/.fozzy/aegis"
SEED="424242"
DOCTOR_RUNS="5"
CORE_SCENARIO="tests/aegis_core.fozzy.json"
HOST_SCENARIO="tests/aegis_host_backed.fozzy.json"
DOCTOR_SCENARIO="tests/aegis_native_doctor.fozzy.json"
FUZZ_SCENARIO="tests/aegis_native_server_main_state_config_executor_host_memory_fuzz.fozzy.json"
EXPLORE_SCENARIO="tests/aegis_native_server_main_state_config_executor_explore.fozzy.json"
SHRINK_SCENARIO="tests/aegis_native_server_main_state_config_executor_fail_shrink.fozzy.json"
CORE_TRACE="${OUT_DIR}/core.det.trace.fozzy"
HOST_TRACE="${OUT_DIR}/host.det.trace.fozzy"
EXPLORE_TRACE="${OUT_DIR}/explore.trace.fozzy"
FAIL_TRACE="${OUT_DIR}/fail.trace.fozzy"
MIN_TRACE="${OUT_DIR}/fail.min.fozzy"

cd "${ROOT_DIR}"
mkdir -p "${OUT_DIR}"
rm -f "${HOST_TRACE}" "${EXPLORE_TRACE}" "${FAIL_TRACE}" "${MIN_TRACE}"

run_json() {
  local output_path="$1"
  shift
  "$@" --json > "${output_path}"
}

echo "[fozzy] env"
run_json "${OUT_DIR}/env.json" fozzy env --strict

echo "[fozzy] usage"
fozzy usage --strict > "${OUT_DIR}/usage.txt"

echo "[fozzy] version"
run_json "${OUT_DIR}/version.json" fozzy version --strict

echo "[fozzy] suite map"
run_json "${OUT_DIR}/map.suites.json" \
  fozzy map suites --root . --scenario-root tests --profile pedantic --strict

for scenario in \
  "${CORE_SCENARIO}" \
  "${HOST_SCENARIO}" \
  "${DOCTOR_SCENARIO}" \
  "${FUZZ_SCENARIO}" \
  "${SHRINK_SCENARIO}"; do
  name="$(basename "${scenario}" .fozzy.json)"
  echo "[fozzy] validate ${scenario}"
  run_json "${OUT_DIR}/${name}.validate.json" fozzy validate "${scenario}" --strict
done

echo "[fozzy] deterministic doctor core"
run_json "${OUT_DIR}/doctor.core.deep.json" \
  fozzy doctor --deep --scenario "${CORE_SCENARIO}" --runs "${DOCTOR_RUNS}" --seed "${SEED}" --strict

echo "[fozzy] deterministic doctor host-backed"
run_json "${OUT_DIR}/doctor.host.deep.json" \
  fozzy doctor --deep --scenario "${HOST_SCENARIO}" --runs "${DOCTOR_RUNS}" --seed "${SEED}" --strict \
    --proc-backend host \
    --http-backend host \
    --fs-backend host

echo "[fozzy] deterministic tests"
run_json "${OUT_DIR}/test.det.json" \
  fozzy test "${CORE_SCENARIO}" "${DOCTOR_SCENARIO}" "${FUZZ_SCENARIO}" --det --strict-verify --seed "${SEED}"

echo "[fozzy] deterministic host-backed tests"
run_json "${OUT_DIR}/test.host.det.json" \
  fozzy test "${HOST_SCENARIO}" --det --strict-verify --seed "${SEED}" \
    --proc-backend host \
    --http-backend host \
    --fs-backend host

echo "[fozzy] host-backed deterministic trace"
run_json "${OUT_DIR}/host.run.json" \
  fozzy run "${HOST_SCENARIO}" --det --strict-verify --seed "${SEED}" \
    --proc-backend host \
    --http-backend host \
    --fs-backend host \
    --record "${HOST_TRACE}"

echo "[fozzy] verify/replay/ci host-backed trace"
run_json "${OUT_DIR}/host.trace.verify.json" \
  fozzy trace verify "${HOST_TRACE}" --strict-verify \
    --proc-backend host \
    --http-backend host \
    --fs-backend host
run_json "${OUT_DIR}/host.replay.json" \
  fozzy replay "${HOST_TRACE}" --strict-verify \
    --proc-backend host \
    --http-backend host \
    --fs-backend host
run_json "${OUT_DIR}/host.ci.json" \
  fozzy ci "${HOST_TRACE}" --strict-verify \
    --proc-backend host \
    --http-backend host \
    --fs-backend host

echo "[fozzy] native doctor surface"
run_json "${OUT_DIR}/doctor.run.json" \
  fozzy run "${DOCTOR_SCENARIO}" --strict-verify \
    --proc-backend host \
    --http-backend host \
    --fs-backend host

echo "[fozzy] memory/report coverage run"
run_json "${OUT_DIR}/fuzz-signal.run.json" \
  fozzy run "${FUZZ_SCENARIO}" --strict-verify \
    --proc-backend host \
    --http-backend host \
    --fs-backend host \
    --mem-track \
    --mem-artifacts

echo "[fozzy] report/memory/artifacts on deterministic trace"
run_json "${OUT_DIR}/latest.report.json" fozzy report show latest --format json
run_json "${OUT_DIR}/latest.artifacts.ls.json" fozzy artifacts ls latest

echo "[fozzy] full gate passed"
