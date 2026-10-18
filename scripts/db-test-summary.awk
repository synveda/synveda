# Summarises a `scripts/db-test.sh` workspace log: per-binary results, every
# ignored case with whether a serial phase executed it, and, for the full gate,
# the floor that proves database acceptance actually ran.
#
#   awk -v full=1 -v required="authz_hierarchy rls" -f scripts/db-test-summary.awk LOG
#
# `required` names integration suites (`tests/<name>.rs`) that must pass at
# least one case. With `full=1`, a case ignored for a serial reason must also
# pass in a later phase. Output carries test names and counts only.

function binary_of(line, path) {
  path = line
  sub(/^.*\(/, "", path)
  sub(/\)[ \t]*$/, "", path)
  gsub(/\\/, "/", path)
  sub(/^.*\//, "", path)
  sub(/\.exe$/, "", path)
  sub(/-[0-9a-f]+$/, "", path)
  return path
}

function source_of(line, path) {
  path = line
  sub(/^[ \t]*Running[ \t]+/, "", path)
  sub(/^unittests[ \t]+/, "", path)
  sub(/[ \t]+\(.*$/, "", path)
  gsub(/\\/, "/", path)
  return path
}

# One `N label` count from a libtest `test result:` line.
function result_count(line, label, parts, n, i, part) {
  n = split(line, parts, ";")
  for (i = 1; i <= n; i++) {
    part = parts[i]
    if (sub(" " label "$", "", part)) {
      sub(/^.*[^0-9]/, "", part)
      return part + 0
    }
  }
  return 0
}

/^[ \t]*Running[ \t]/ {
  binary = binary_of($0)
  current = binary " " source_of($0)
  next
}

/^[ \t]*Doc-tests[ \t]/ {
  binary = "doc-" $2
  current = binary
  next
}

/^test .* \.\.\. / {
  name = $2
  key = binary SUBSEP name
  if ($0 ~ / \.\.\. ignored/) {
    reason = $0
    if (!sub(/^.* \.\.\. ignored, /, "", reason)) reason = "no reason given"
    if (!(key in ignored)) {
      ignored[key] = reason
      ignored_order[++ignored_count] = key
    }
  } else if ($0 ~ / \.\.\. ok$/) {
    passed_case[key] = 1
  }
  next
}

/^test result: / {
  if (!(current in seen)) {
    seen[current] = 1
    suites[++suite_count] = current
  }
  passed[current] += result_count($0, "passed")
  failed[current] += result_count($0, "failed")
  skipped[current] += result_count($0, "ignored")
  next
}

END {
  status = 0
  print "db-test: executed test binaries (passed/failed/ignored):"
  for (i = 1; i <= suite_count; i++) {
    suite = suites[i]
    printf "  %s %d/%d/%d\n", suite, passed[suite], failed[suite], skipped[suite]
    total_passed += passed[suite]
    total_failed += failed[suite]
    total_ignored += skipped[suite]
  }
  printf "db-test: %d binaries, %d passed, %d failed, %d ignored\n", \
    suite_count, total_passed, total_failed, total_ignored
  for (i = 1; i <= ignored_count; i++) {
    key = ignored_order[i]
    split(key, parts, SUBSEP)
    if (key in passed_case) {
      printf "db-test: ignored, then executed serially: %s %s\n", parts[1], parts[2]
      continue
    }
    printf "db-test: ignored and not executed here: %s %s (%s)\n", parts[1], parts[2], ignored[key]
    if (full && ignored[key] ~ /serial/) {
      printf "db-test: serial case was never executed: %s %s\n", parts[1], parts[2] > "/dev/stderr"
      status = 1
    }
  }
  if (total_failed) {
    print "db-test: the log records failed cases" > "/dev/stderr"
    status = 1
  }
  if (suite_count == 0) {
    print "db-test: the log records no executed test binary" > "/dev/stderr"
    status = 1
  }
  n = split(required, names, " ")
  for (r = 1; r <= n; r++) {
    found = 0
    for (i = 1; i <= suite_count; i++) {
      split(suites[i], words, " ")
      if (words[2] == "tests/" names[r] ".rs" && passed[suites[i]] > 0) found = 1
    }
    if (!found) {
      printf "db-test: required database suite executed no case: %s\n", names[r] > "/dev/stderr"
      status = 1
    }
  }
  exit status
}
