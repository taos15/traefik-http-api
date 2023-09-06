import { traefik } from "../../config/traefikConfigTemplate";
import { getFilteredServices } from "./getFilteredServices";

export async function addServicesToTraefik() {
    let filteredServices = await getFilteredServices();
    if (filteredServices) {
        Object.assign(traefik.http.services, ...filteredServices);
    }
}
