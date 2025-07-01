#! /bin/bash

export DATA_DIR=${DATA_DIR:-/convex/data}
export TMPDIR=${TMPDIR:-"$DATA_DIR/tmp"}
export STORAGE_DIR=${STORAGE_DIR:-"$DATA_DIR/storage"}
export SQLITE_DB=${SQLITE_DB:-"$DATA_DIR/db.sqlite3"}

# Database driver flags matching DbDriverTag values
POSTGRES_DB_FLAGS=(--db postgres-v5)
MYSQL_DB_FLAGS=(--db mysql-v5)

set -e
mkdir -p "$TMPDIR" "$STORAGE_DIR"

source ./read_credentials.sh

# Determine database configuration
if [ -n "$POSTGRES_URL" ]; then
    DB_SPEC="$POSTGRES_URL"
    DB_FLAGS=("${POSTGRES_DB_FLAGS[@]}")
elif [ -n "$MYSQL_URL" ]; then
    DB_SPEC="$MYSQL_URL"
    DB_FLAGS=("${MYSQL_DB_FLAGS[@]}")
elif [ -n "$DATABASE_URL" ]; then
    echo "Warning: DATABASE_URL is deprecated. Please use POSTGRES_URL for PostgreSQL or MYSQL_URL for MySQL connections instead."
    DB_SPEC="$DATABASE_URL"
    DB_FLAGS=("${POSTGRES_DB_FLAGS[@]}")  # Maintain backwards compatibility with existing DATABASE_URL behavior
else
    # Otherwise fallback to SQLite
    DB_SPEC="$SQLITE_DB"
    DB_FLAGS=()
fi

# Check if all required S3 environment variables are present
MISSING_VARS=()
[ -z "$AWS_REGION" ] && MISSING_VARS+=("AWS_REGION")
[ -z "$AWS_ACCESS_KEY_ID" ] && MISSING_VARS+=("AWS_ACCESS_KEY_ID")
[ -z "$AWS_SECRET_ACCESS_KEY" ] && MISSING_VARS+=("AWS_SECRET_ACCESS_KEY")
[ -z "$S3_STORAGE_EXPORTS_BUCKET" ] && MISSING_VARS+=("S3_STORAGE_EXPORTS_BUCKET")
[ -z "$S3_STORAGE_SNAPSHOT_IMPORTS_BUCKET" ] && MISSING_VARS+=("S3_STORAGE_SNAPSHOT_IMPORTS_BUCKET")
[ -z "$S3_STORAGE_MODULES_BUCKET" ] && MISSING_VARS+=("S3_STORAGE_MODULES_BUCKET")
[ -z "$S3_STORAGE_FILES_BUCKET" ] && MISSING_VARS+=("S3_STORAGE_FILES_BUCKET")
[ -z "$S3_STORAGE_SEARCH_BUCKET" ] && MISSING_VARS+=("S3_STORAGE_SEARCH_BUCKET")

if [ ${#MISSING_VARS[@]} -eq 0 ]; then
    STORAGE_FLAGS=(--s3-storage)
else
    if [ ${#MISSING_VARS[@]} -lt 8 ]; then
        echo "Warning: Some AWS/S3 environment variables are missing. Falling back to local storage."
        echo "Missing variables: ${MISSING_VARS[*]}"
    fi
    STORAGE_FLAGS=(--local-storage "$STORAGE_DIR")
fi

# --port and --site-proxy-port are internal to the container, so we pick them to
# avoid conflicts in the container.
# --convex-origin and --convex-site are how the backend can be contacted from
# the outside world. They show up in storage urls, action callbacks, etc.

exec ./convex-local-backend "$@" \
    --instance-name "$INSTANCE_NAME" \
    --instance-secret "$INSTANCE_SECRET" \
    --port 3210 \
    --site-proxy-port 3211 \
    --convex-origin "$CONVEX_CLOUD_ORIGIN" \
    --convex-site "$CONVEX_SITE_ORIGIN" \
    --beacon-tag "self-hosted-docker" \
    ${DISABLE_BEACON:+--disable-beacon} \
    ${REDACT_LOGS_TO_CLIENT:+--redact-logs-to-client} \
    ${DO_NOT_REQUIRE_SSL:+--do-not-require-ssl} \
    "${DB_FLAGS[@]}" \
    "${STORAGE_FLAGS[@]}" \
    "$DB_SPEC"