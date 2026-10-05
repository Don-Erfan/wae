// A shared package must not depend on the app that consumes it (PACKAGE-001/002/003).
import { routes } from "@synthetic/web/routes";
export { Button } from "./button";

export const home = routes.home;
