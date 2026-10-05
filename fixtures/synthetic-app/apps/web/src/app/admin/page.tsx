// Pages must reach the database through a feature, not directly (ARCH-002).
import { db } from "@/server/db";

export default function AdminPage() {
  return db;
}
