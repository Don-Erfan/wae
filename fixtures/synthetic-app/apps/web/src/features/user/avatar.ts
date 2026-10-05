// Uses a Node builtin; browser callers reach it through the user entrypoint (RUNTIME-002).
import { join } from "node:path";

export const avatarPath = (id: string) => join("/avatars", id);
