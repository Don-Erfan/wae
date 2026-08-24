import type { Domain } from "@fixture/domain";
import { publicValue } from "@fixture/domain/public";
export const value: Domain | undefined = publicValue ? undefined : undefined;
