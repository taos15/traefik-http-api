import { traefik } from "./config/traefikConfigTemplate";
import { getFilteredRoutes } from "./getFilteredRoutes";

export async function addRoutesToTraefik() {
    const filteredRoutes = await getFilteredRoutes();
    Object.assign(traefik.http.routers, ...filteredRoutes);
}
