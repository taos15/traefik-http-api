import { traefik } from "./config/traefikConfigTemplate";
import { getFilteredServices } from "./getFilteredServices";

export async function addServicesToTraefik() {
    const filteredServices = await getFilteredServices();
    Object.assign(traefik.http.services, ...filteredServices);
}
