"use client";

// Client component: reaches server-only and Node-only code through the user feature
// (RUNTIME-001, RUNTIME-002, RUNTIME-003).
import { avatarPath, getProfile } from "@/features/user";

export function ProfileCard() {
  return [avatarPath("me"), getProfile()];
}
