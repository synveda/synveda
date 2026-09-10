#!/bin/sh
# Shared exact-project exclusion for Compose lifecycle and authority-file writers.
# The caller must set absolute compose_dir and validated project first.

project_lock_owned=false
project_lock_inherited=false
project_lock_claim_file=
project_lock_claim_identity=
project_lock_identity=
project_lock_root=/tmp/.synveda-compose-locks-$(id -u)
project_lock_file=$project_lock_root/$project.lock

project_lock_mode() {
    stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1" 2>/dev/null
}

project_lock_owner_id() {
    stat -c '%u' "$1" 2>/dev/null || stat -f '%u' "$1" 2>/dev/null
}

project_lock_file_identity() {
    stat -c '%d:%i' "$1" 2>/dev/null || stat -f '%d:%i' "$1" 2>/dev/null
}

require_project_lock_root() {
    [ ! -L "$project_lock_root" ] && [ -d "$project_lock_root" ] && \
        [ "$(project_lock_mode "$project_lock_root")" = 700 ] && \
        [ "$(project_lock_owner_id "$project_lock_root")" = "$(id -u)" ] || {
        echo "project-lock: private runtime directory metadata was refused" >&2
        return 73
    }
}

ensure_project_lock_root() {
    if [ -e "$project_lock_root" ] || [ -L "$project_lock_root" ]; then
        require_project_lock_root
        return
    fi
    if ! mkdir -m 700 -- "$project_lock_root" 2>/dev/null; then
        require_project_lock_root
        return
    fi
    chmod 700 "$project_lock_root"
    require_project_lock_root
}

require_project_lock_file() {
    required_lock_file=$1
    required_lock_owner=$2
    [ ! -L "$required_lock_file" ] && [ -f "$required_lock_file" ] && \
        [ "$(project_lock_mode "$required_lock_file")" = 600 ] && \
        [ "$(project_lock_owner_id "$required_lock_file")" = "$(id -u)" ] || {
        echo "project-lock: lock owner metadata was refused" >&2
        return 73
    }
    recorded_lock_owner=
    IFS= read -r recorded_lock_owner < "$required_lock_file" || recorded_lock_owner=
    [ "$recorded_lock_owner" = "$required_lock_owner" ] || {
        echo "project-lock: lock owner was refused" >&2
        return 73
    }
}

discard_project_lock_claim() {
    [ -n "$project_lock_claim_file" ] || return 0
    if [ -e "$project_lock_claim_file" ] || [ -L "$project_lock_claim_file" ]; then
        require_project_lock_file "$project_lock_claim_file" \
            "${project_lock_owner:-}" || return 73
        if [ -n "$project_lock_claim_identity" ]; then
            [ "$(project_lock_file_identity "$project_lock_claim_file")" = \
                "$project_lock_claim_identity" ] || return 73
        fi
        rm -f -- "$project_lock_claim_file" || return 73
    fi
    project_lock_claim_file=
    project_lock_claim_identity=
}

acquire_project_lock() {
    case "${SYNVEDA_INTERNAL_PROJECT_LOCK_FILE+x}:${SYNVEDA_INTERNAL_PROJECT_LOCK_OWNER+x}" in
        :) ;;
        x:x)
            [ "$SYNVEDA_INTERNAL_PROJECT_LOCK_FILE" = "$project_lock_file" ] || {
                echo "project-lock: inherited lock path was refused" >&2
                return 73
            }
            case "$SYNVEDA_INTERNAL_PROJECT_LOCK_OWNER" in
                "$project":*) inherited_lock_pid=${SYNVEDA_INTERNAL_PROJECT_LOCK_OWNER#"$project":} ;;
                *) inherited_lock_pid=invalid ;;
            esac
            case "$inherited_lock_pid" in
                ''|0|0*|*[!0-9]*) inherited_lock_pid=invalid ;;
            esac
            # Bounded lifecycle children sit below the deadline runner, so the
            # lock owner is an ancestor rather than necessarily the immediate
            # parent. The exact marker plus a live owner remains the capability;
            # same-UID processes could unlink the advisory file regardless.
            [ "$inherited_lock_pid" != invalid ] && \
                require_project_lock_file "$project_lock_file" \
                    "$SYNVEDA_INTERNAL_PROJECT_LOCK_OWNER" && \
                kill -0 "$inherited_lock_pid" 2>/dev/null || {
                echo "project-lock: inherited lock owner was refused" >&2
                return 73
            }
            project_lock_inherited=true
            return
            ;;
        *)
            echo "project-lock: partial inherited lock state was refused" >&2
            return 73
            ;;
    esac

    old_umask=$(umask)
    umask 077
    ensure_project_lock_root || {
        umask "$old_umask"
        return 73
    }
    project_lock_owner=$project:$$
    project_lock_claim_file=$(mktemp \
        "$project_lock_root/.$project.claim.XXXXXX") || {
        umask "$old_umask"
        echo "project-lock: lock claim could not be staged" >&2
        return 73
    }
    chmod 600 "$project_lock_claim_file" || {
        umask "$old_umask"
        echo "project-lock: lock claim could not be staged" >&2
        return 73
    }
    if ! printf '%s\n' "$project_lock_owner" > "$project_lock_claim_file" || \
        ! require_project_lock_file "$project_lock_claim_file" "$project_lock_owner"; then
        discard_project_lock_claim 2>/dev/null || true
        umask "$old_umask"
        echo "project-lock: lock owner could not be recorded" >&2
        return 73
    fi
    project_lock_claim_identity=$(project_lock_file_identity \
        "$project_lock_claim_file") || {
        umask "$old_umask"
        echo "project-lock: lock claim identity was unavailable" >&2
        return 73
    }
    if ! ln "$project_lock_claim_file" "$project_lock_file" 2>/dev/null; then
        discard_project_lock_claim 2>/dev/null || true
        umask "$old_umask"
        echo "project-lock: another lifecycle or authority action owns $project" >&2
        return 75
    fi
    if ! require_project_lock_file "$project_lock_file" "$project_lock_owner" || \
        [ "$(project_lock_file_identity "$project_lock_file" 2>/dev/null || true)" != \
            "$project_lock_claim_identity" ]; then
        umask "$old_umask"
        echo "project-lock: lock publication state was refused" >&2
        return 73
    fi
    project_lock_identity=$project_lock_claim_identity
    project_lock_owned=true
    if ! discard_project_lock_claim; then
        umask "$old_umask"
        echo "project-lock: private lock claim cleanup failed" >&2
        return 73
    fi
    umask "$old_umask"
    SYNVEDA_INTERNAL_PROJECT_LOCK_FILE=$project_lock_file
    SYNVEDA_INTERNAL_PROJECT_LOCK_OWNER=$project_lock_owner
    export SYNVEDA_INTERNAL_PROJECT_LOCK_FILE SYNVEDA_INTERNAL_PROJECT_LOCK_OWNER
}

release_project_lock() {
    if [ "$project_lock_owned" != true ] && \
        [ -n "$project_lock_claim_identity" ] && \
        [ ! -L "$project_lock_file" ] && [ -f "$project_lock_file" ] && \
        [ "$(project_lock_file_identity "$project_lock_file" 2>/dev/null || true)" = \
            "$project_lock_claim_identity" ]; then
        require_project_lock_file "$project_lock_file" \
            "${project_lock_owner:-}" || return 73
        project_lock_identity=$project_lock_claim_identity
        project_lock_owned=true
    fi
    if [ "$project_lock_owned" = true ]; then
        require_project_lock_file "$project_lock_file" "$project_lock_owner" || return 73
        [ "$(project_lock_file_identity "$project_lock_file")" = \
            "$project_lock_identity" ] || {
            echo "project-lock: lock ownership changed; refusing release" >&2
            return 73
        }
        rm -f -- "$project_lock_file" || return 73
        project_lock_owned=false
    fi
    discard_project_lock_claim || return 73
    unset SYNVEDA_INTERNAL_PROJECT_LOCK_FILE SYNVEDA_INTERNAL_PROJECT_LOCK_OWNER
}
