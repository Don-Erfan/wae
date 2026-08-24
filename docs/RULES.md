# WAE rule reference

## ARCH-001

Detects circular dependencies in the analyzed module graph.

## ARCH-002

Enforces configured forbidden-dependency policies and optional architecture presets.

## ARCH-003

Enforces configured layer import permissions. A module matching multiple layers is a configuration error.

## ARCH-004

Requires every importer outside a feature's owning package/feature to use that feature's public entrypoint.

## ARCH-005

Rejects imports of explicitly private path segments from outside their owning package/feature.
