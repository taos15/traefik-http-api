import { traefik } from "../../config/traefikConfigTemplate";
import { getFilteredRoutes } from "./getFilteredRoutes";

export async function addRoutesToTraefik() {
    let filteredRoutes = await getFilteredRoutes();
    if (filteredRoutes) {
        Object.assign(traefik.http.routers, ...filteredRoutes);
    }
}
