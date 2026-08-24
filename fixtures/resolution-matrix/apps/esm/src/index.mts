import type { Domain } from "@fixture/domain";
import { publicValue } from "@fixture/domain/public";
import { local } from "./local.js";
export const value: Domain | undefined = publicValue && local ? undefined : undefined;
