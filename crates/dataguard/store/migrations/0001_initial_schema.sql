-- 0001 initial schema of the Data layer: DataQueue, dataVersion counter,
-- audit log, DataCapability registry, and the grants to the two roles of #73.
--
-- Runs in the runner's single transaction: any failure leaves nothing behind.
-- Forward-only. Never edit once applied: the runner refuses a changed checksum.
-- Objects land in the connection's current schema; no name is qualified.
-- No SEQUENCE (a rollback would leave a gap in dataVersion), no trigger and no
-- function: shapes are enforced by constraints, immutability by privileges the
-- owner revokes from itself at the end of this file.
-- A CHECK that evaluates to NULL passes, so every test on a JSON member is
-- wrapped in coalesce(..., false): a missing member must fail the check.

-- Roles are created by #73, never here. A missing one fails the migration and
-- the message names it.
DO $$
DECLARE
    role_name text;
BEGIN
    FOREACH role_name IN ARRAY ARRAY['dnd_app', 'dnd_readonly'] LOOP
        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = role_name) THEN
            RAISE EXCEPTION 'role % does not exist', role_name
                USING ERRCODE = 'undefined_object';
        END IF;
    END LOOP;
END
$$;

-- DataCapability registry (contract F). A row is immutable once written.
CREATE TABLE data_capability_registry (
    name text NOT NULL
        CONSTRAINT data_capability_registry_name_check
        CHECK (name ~ '^[a-z][a-z0-9-]*\.[A-Za-z][A-Za-z0-9]*$'),
    version integer NOT NULL
        CONSTRAINT data_capability_registry_version_check CHECK (version >= 1),
    -- The full id `system.name@version`: the target of the queue's foreign key.
    ref text GENERATED ALWAYS AS (name || '@' || version::text) STORED,
    owner text NOT NULL
        CONSTRAINT data_capability_registry_owner_check CHECK (owner = 'dataguard'),
    effect text NOT NULL
        CONSTRAINT data_capability_registry_effect_check
        CHECK (effect IN ('insert', 'update', 'delete', 'upsert')),
    target jsonb NOT NULL
        CONSTRAINT data_capability_registry_target_check
        CHECK (
            jsonb_typeof(target) = 'object'
            AND coalesce(target ->> 'aggregate' ~ '^[A-Z][A-Za-z0-9]*$', false)
        ),
    touches jsonb NOT NULL
        CONSTRAINT data_capability_registry_touches_check
        CHECK (jsonb_typeof(touches) = 'array'),
    mode text NOT NULL
        CONSTRAINT data_capability_registry_mode_check
        CHECK (mode IN ('confirm_on_stale', 'overwrite', 'relative')),
    payload jsonb NOT NULL
        CONSTRAINT data_capability_registry_payload_check
        CHECK (jsonb_typeof(payload) = 'object'),
    invariants jsonb NOT NULL
        CONSTRAINT data_capability_registry_invariants_check
        CHECK (jsonb_typeof(invariants) = 'array'),
    permissions jsonb NOT NULL
        CONSTRAINT data_capability_registry_permissions_check
        CHECK (jsonb_typeof(permissions) = 'array'),
    callable_by jsonb NOT NULL
        CONSTRAINT data_capability_registry_callable_by_check
        CHECK (jsonb_typeof(callable_by) = 'array'),
    idempotency_key text NOT NULL
        CONSTRAINT data_capability_registry_idempotency_key_check
        CHECK (idempotency_key IN ('required', 'optional')),
    -- The whole manifest, kept for content comparison on re-registration.
    manifest jsonb NOT NULL
        CONSTRAINT data_capability_registry_manifest_check
        CHECK (jsonb_typeof(manifest) = 'object'),
    registered_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT data_capability_registry_pkey PRIMARY KEY (name, version),
    CONSTRAINT data_capability_registry_ref_key UNIQUE (ref)
);

-- DataQueue: one row per command, with the fields of contract G plus the
-- columns G lacks (payload, idempotency_key, data_version, violations,
-- created_at). No hold column, no hold state.
CREATE TABLE data_queue (
    command text NOT NULL
        CONSTRAINT data_queue_command_check CHECK (command <> ''),
    data_capability text NOT NULL
        CONSTRAINT data_queue_data_capability_check
        CHECK (data_capability ~ '^[a-z][a-z0-9-]*\.[A-Za-z][A-Za-z0-9]*@[0-9]+$')
        CONSTRAINT data_queue_data_capability_fkey
        REFERENCES data_capability_registry (ref) ON DELETE RESTRICT ON UPDATE RESTRICT,
    by text NOT NULL
        CONSTRAINT data_queue_by_check CHECK (by <> ''),
    -- `<Aggregate>/<id>`: the first `/` separates the two, the id is not empty.
    partition text NOT NULL
        CONSTRAINT data_queue_partition_check
        CHECK (
            -- The id excludes the line terminators ECMA's `.` excludes, which
            -- PostgreSQL's `.` would accept (contract G).
            partition ~ '^[A-Z][A-Za-z0-9]*/[^\n\r\u2028\u2029]+$'
            AND octet_length(partition) <= 512
        ),
    -- No default: the enqueue assigns it. Gaps are allowed.
    position bigint NOT NULL
        CONSTRAINT data_queue_position_check CHECK (position >= 0),
    based_on jsonb NOT NULL
        CONSTRAINT data_queue_based_on_check
        CHECK (
            jsonb_typeof(based_on) = 'object'
            AND coalesce(jsonb_typeof(based_on -> 'version') = 'number', false)
            AND coalesce(based_on ->> 'version' ~ '^[0-9]{1,18}$', false)
            AND (NOT (based_on ? 'values') OR jsonb_typeof(based_on -> 'values') = 'object')
            AND (based_on - 'version' - 'values') = '{}'::jsonb
        ),
    state text NOT NULL DEFAULT 'queued'
        CONSTRAINT data_queue_state_check
        CHECK (state IN (
            'queued', 'awaiting_confirmation', 'confirmed', 'awaiting_review',
            'parked', 'applied', 'rejected', 'cancelled', 'expired'
        )),
    your_value jsonb,
    projection jsonb
        CONSTRAINT data_queue_projection_check
        CHECK (
            projection IS NULL
            OR (
                jsonb_typeof(projection) = 'object'
                AND coalesce(jsonb_typeof(projection -> 'confirmed') = 'object', false)
                AND coalesce(jsonb_typeof(projection -> 'pendingAhead') = 'array', false)
                AND (projection - 'confirmed' - 'pendingAhead') = '{}'::jsonb
            )
        ),
    confirmation jsonb
        CONSTRAINT data_queue_confirmation_check
        CHECK (
            confirmation IS NULL
            OR (
                jsonb_typeof(confirmation) = 'object'
                AND coalesce(jsonb_typeof(confirmation -> 'by') = 'string', false)
            )
        ),
    parked jsonb
        CONSTRAINT data_queue_parked_check
        CHECK (
            parked IS NULL
            OR (
                jsonb_typeof(parked) = 'object'
                AND coalesce(jsonb_typeof(parked -> 'ttl') = 'string', false)
                AND coalesce(parked ->> 'ttl' ~ '^[0-9]+(ms|s|m|h|d)$', false)
                AND coalesce(parked ->> 'onExpire' = 'drop', false)
            )
        ),
    requeued_from text
        CONSTRAINT data_queue_requeued_from_check
        CHECK (requeued_from IS NULL OR requeued_from <> command)
        CONSTRAINT data_queue_requeued_from_fkey
        REFERENCES data_queue (command) ON DELETE RESTRICT ON UPDATE RESTRICT,
    payload jsonb NOT NULL
        CONSTRAINT data_queue_payload_check CHECK (jsonb_typeof(payload) = 'object'),
    idempotency_key text
        CONSTRAINT data_queue_idempotency_key_check
        CHECK (idempotency_key IS NULL OR idempotency_key <> ''),
    created_at timestamptz NOT NULL DEFAULT now(),
    data_version bigint
        CONSTRAINT data_queue_data_version_check
        CHECK (data_version IS NULL OR data_version >= 1),
    violations jsonb
        CONSTRAINT data_queue_violations_check
        CHECK (violations IS NULL OR jsonb_typeof(violations) = 'array'),
    CONSTRAINT data_queue_pkey PRIMARY KEY (command),
    -- Not deferrable: a position is never held by two commands, not even
    -- within a transaction.
    CONSTRAINT data_queue_partition_position_key UNIQUE (partition, position),
    -- Contract H: the result a terminal state can carry.
    CONSTRAINT data_queue_applied_result_check
        CHECK (
            state <> 'applied'
            OR (data_version IS NOT NULL AND (violations IS NULL OR violations = '[]'::jsonb))
        ),
    CONSTRAINT data_queue_rejected_result_check
        CHECK (
            state <> 'rejected'
            OR (
                data_version IS NULL
                AND violations IS NOT NULL
                AND jsonb_typeof(violations) = 'array'
                AND violations <> '[]'::jsonb
            )
        ),
    -- Only an applied command has a dataVersion; with the check above this
    -- makes `state = 'applied'` and `data_version IS NOT NULL` equivalent.
    CONSTRAINT data_queue_only_applied_has_version_check
        CHECK (state = 'applied' OR data_version IS NULL)
);

-- One applied dataVersion is never given to two commands.
CREATE UNIQUE INDEX data_queue_data_version_key
    ON data_queue (data_version) WHERE data_version IS NOT NULL;

-- A key is unique per exact capability version; null keys never collide.
CREATE UNIQUE INDEX data_queue_idempotency_key_key
    ON data_queue (data_capability, idempotency_key) WHERE idempotency_key IS NOT NULL;

-- The gapless dataVersion counter: one row, a table and not a sequence.
CREATE TABLE data_version_counter (
    id boolean NOT NULL DEFAULT true
        CONSTRAINT data_version_counter_single_row_check CHECK (id),
    value bigint NOT NULL
        CONSTRAINT data_version_counter_value_check CHECK (value >= 0),
    CONSTRAINT data_version_counter_pkey PRIMARY KEY (id)
);

-- Runs once: the runner never replays an applied migration.
INSERT INTO data_version_counter (id, value) VALUES (true, 0);

-- Audit log: append-only, one row per event.
CREATE TABLE audit_log (
    id uuid NOT NULL DEFAULT gen_random_uuid(),
    event text NOT NULL
        CONSTRAINT audit_log_event_check
        CHECK (event IN ('enqueued', 'applied', 'rejected', 'cancelled', 'refused')),
    -- Null only for `refused`: a pre-enqueue refusal has no queue row.
    command text
        CONSTRAINT audit_log_command_fkey
        REFERENCES data_queue (command) ON DELETE RESTRICT ON UPDATE RESTRICT,
    actor text NOT NULL
        CONSTRAINT audit_log_actor_check CHECK (actor <> ''),
    -- Plain text, no foreign key: a refusal may name an unknown capability.
    data_capability text NOT NULL,
    at timestamptz NOT NULL DEFAULT clock_timestamp(),
    detail jsonb NOT NULL DEFAULT '{}'::jsonb
        CONSTRAINT audit_log_detail_check CHECK (jsonb_typeof(detail) = 'object'),
    data_version bigint
        CONSTRAINT audit_log_data_version_check
        CHECK (data_version IS NULL OR data_version >= 1),
    CONSTRAINT audit_log_pkey PRIMARY KEY (id),
    CONSTRAINT audit_log_command_required_check
        CHECK (event = 'refused' OR command IS NOT NULL),
    CONSTRAINT audit_log_data_version_iff_applied_check
        CHECK ((event = 'applied') = (data_version IS NOT NULL))
);

-- Privileges. The roles hold USAGE on the schema and nothing else on it.
DO $$
BEGIN
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO dnd_app, dnd_readonly', current_schema());
END
$$;

-- Each table starts from nothing. The REVOKE from dnd_app is the owner (the
-- migrating role) taking its own privileges away: that is what makes the
-- immutability rules bind it, with no trigger. No DELETE, TRUNCATE,
-- REFERENCES or TRIGGER for anyone; PUBLIC holds nothing.
REVOKE ALL ON data_queue FROM PUBLIC, dnd_app, dnd_readonly;
GRANT SELECT ON data_queue TO dnd_readonly;
GRANT SELECT, INSERT ON data_queue TO dnd_app;
-- Only the mutable columns: command, partition, position, data_capability,
-- by, payload, idempotency_key and created_at keep a command in its place.
GRANT UPDATE (
    state, based_on, projection, your_value, confirmation, parked,
    requeued_from, data_version, violations
) ON data_queue TO dnd_app;

REVOKE ALL ON data_version_counter FROM PUBLIC, dnd_app, dnd_readonly;
GRANT SELECT ON data_version_counter TO dnd_readonly;
GRANT SELECT ON data_version_counter TO dnd_app;
GRANT UPDATE (value) ON data_version_counter TO dnd_app;

REVOKE ALL ON audit_log FROM PUBLIC, dnd_app, dnd_readonly;
GRANT SELECT ON audit_log TO dnd_readonly;
GRANT SELECT, INSERT ON audit_log TO dnd_app;

REVOKE ALL ON data_capability_registry FROM PUBLIC, dnd_app, dnd_readonly;
GRANT SELECT ON data_capability_registry TO dnd_readonly;
GRANT SELECT, INSERT ON data_capability_registry TO dnd_app;
-- PostgreSQL checks a foreign key as the referenced table's owner, with
-- FOR KEY SHARE, which needs UPDATE on at least one column of that table.
-- `ref` is generated: any UPDATE that sets it to a value fails, so no content
-- of a registered row can change.
GRANT UPDATE (ref) ON data_capability_registry TO dnd_app;

-- The runner records every later migration as dnd_app.
REVOKE ALL ON schema_migrations FROM PUBLIC, dnd_app, dnd_readonly;
GRANT SELECT ON schema_migrations TO dnd_readonly;
GRANT SELECT, INSERT ON schema_migrations TO dnd_app;
