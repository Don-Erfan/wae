import { type Domain } from "@fixture/domain";
export type { Domain as PublicDomain } from "@fixture/domain";
import { publicValue } from "@fixture/domain/public";
import { local } from "./local.js";
export const value: Domain | undefined = publicValue && local ? undefined : undefined;
