import { Button } from "@synthetic/ui";
import { Cart } from "@/features/cart";
import { ProfileCard } from "./profile/profile-card";

export default function Page() {
  return [Button, Cart, ProfileCard];
}
