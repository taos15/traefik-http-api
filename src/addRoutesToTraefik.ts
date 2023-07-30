import { traefik } from "./config/traefikConfigTemplate";
import { getFilteredRoutes } from "./getFilteredRoutes";

export async function addRoutesToTraefik() {
    const filteredRoutes = await getFilteredRoutes();
    if (filteredRoutes) {
        Object.assign(traefik.http.routers, ...filteredRoutes);
    }
}
