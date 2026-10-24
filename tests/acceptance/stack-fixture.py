"""Generate storage and database fixtures for the disposable application stack."""

import json
import os
import sys


def storage_config(size):
    return {
        "mode": "managed_nfs",
        "mount_path": "/data",
        "uid": 1000,
        "gid": 1000,
        "fs_group": 1000,
        "config_json": {"storage_class": "standard", "size": size},
        "validation_json": {
            "valid": True,
            "message": "Configured by the application test fixture",
            "checks": [],
            "warnings": [],
            "benchmark": None,
        },
    }


def storage_secret(size):
    return {
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {"name": "kubarr-storage-config", "namespace": "kubarr-backend"},
        "type": "Opaque",
        "stringData": {"config.json": json.dumps(storage_config(size))},
    }


def install_request(options):
    if not options.get("app_name") or not options.get("chart_version"):
        raise ValueError("Install options must identify the app and chart version")
    storage = {key: options[key] for key in ("mounts", "config") if options.get(key) is not None}
    if options.get("workloads"):
        storage["workloads"] = {name: {"mounts": mounts} for name, mounts in options["workloads"].items()}
    return {"app_name": options["app_name"], "chart_version": options["chart_version"],
            "storage": storage}


def seed_sql(username, password, email, size):
    def quoted(value):
        return "'" + value.replace("'", "''") + "'"

    config = storage_config(size)
    return f"""BEGIN;
SET LOCAL standard_conforming_strings = on;
CREATE EXTENSION IF NOT EXISTS pgcrypto;
DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM roles WHERE name = 'admin') THEN
    RAISE EXCEPTION 'Admin role migrations are not ready';
  END IF;
END $$;
WITH new_user AS (
  INSERT INTO users (username, email, hashed_password, is_active, is_approved,
                     totp_enabled, created_at, updated_at)
  VALUES ({quoted(username)}, {quoted(email)}, crypt({quoted(password)}, gen_salt('bf', 12)),
          TRUE, TRUE, FALSE, NOW(), NOW())
  RETURNING id
)
INSERT INTO user_roles (user_id, role_id)
SELECT new_user.id, roles.id FROM new_user, roles WHERE roles.name = 'admin';
INSERT INTO storage_config (mode, mount_path, uid, gid, fs_group, config_json,
                            validation_json, created_at, updated_at)
VALUES ('managed_nfs', '/data', 1000, 1000, 1000,
        {quoted(json.dumps(config['config_json']))},
        {quoted(json.dumps(config['validation_json']))}, NOW(), NOW());
COMMIT;
"""


if __name__ == "__main__":
    size = os.environ.get("TEST_STORAGE_SIZE", "2Gi")
    if sys.argv[1:] == ["storage-secret"]:
        print(json.dumps(storage_secret(size)))
    elif sys.argv[1:] == ["seed-sql"]:
        print(seed_sql(os.environ["TEST_USERNAME"], os.environ["TEST_PASSWORD"],
                       os.environ.get("TEST_ADMIN_EMAIL", "admin@example.invalid"), size))
    elif sys.argv[1:] == ["install-request"]:
        print(json.dumps(install_request(json.load(sys.stdin))))
    else:
        sys.exit("Expected storage-secret, seed-sql, or install-request")
