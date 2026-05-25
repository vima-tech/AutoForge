"""
Field-level data masking engine for preview databases.
Design doc §5.3 — applies autoforge.yaml sensitive_fields rules.
"""
import time
import logging
from typing import Any
import psycopg2
import psycopg2.extras

logger = logging.getLogger(__name__)


def _generate_mask_sql(table: str, field: str, rule: str) -> str:
    if rule == "mask":
        # Phone-style: first 3 + **** + last 4
        # Name-style: first char + *
        # We use a generic approach: keep first 3 chars, mask the rest
        return (
            f"UPDATE {table} SET {field} = "
            f"CASE "
            f"  WHEN LENGTH({field}::text) >= 8 THEN "
            f"    CONCAT(LEFT({field}::text, 3), REPEAT('*', LENGTH({field}::text) - 7), RIGHT({field}::text, 4)) "
            f"  WHEN LENGTH({field}::text) > 1 THEN "
            f"    CONCAT(LEFT({field}::text, 1), REPEAT('*', LENGTH({field}::text) - 1)) "
            f"  ELSE {field} "
            f"END "
            f"WHERE {field} IS NOT NULL;"
        )
    elif rule == "hash":
        return f"UPDATE {table} SET {field} = MD5({field}::text) WHERE {field} IS NOT NULL;"
    elif rule == "drop":
        return f"UPDATE {table} SET {field} = NULL;"
    else:
        raise ValueError(f"Unknown masking rule: {rule}")


def apply_masking(*, db_url: str, sensitive_fields: list[dict[str, Any]]) -> dict:
    """
    Apply field-level masking to a preview database.
    Returns audit dict: {field_key: {rows_affected, elapsed_ms}}.
    """
    if not sensitive_fields:
        return {}

    audit: dict[str, Any] = {}

    with psycopg2.connect(db_url) as conn:
        conn.autocommit = True
        with conn.cursor() as cur:
            for spec in sensitive_fields:
                table = spec.get("table")
                fields = spec.get("fields", [])
                rule = spec.get("rule", "mask")

                for field in fields:
                    key = f"{table}.{field}"
                    sql = _generate_mask_sql(table, field, rule)
                    t0 = time.monotonic()
                    try:
                        cur.execute(sql)
                        rows = cur.rowcount
                        elapsed = round((time.monotonic() - t0) * 1000, 1)
                        audit[key] = {"rows_affected": rows, "elapsed_ms": elapsed, "rule": rule}
                        logger.info("Masked %s [%s]: %d rows in %.1fms", key, rule, rows, elapsed)
                    except psycopg2.Error as e:
                        logger.warning("Masking failed for %s: %s", key, e)
                        audit[key] = {"error": str(e)}

    return audit
