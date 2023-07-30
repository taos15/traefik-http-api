import { getAllContainers } from "./getAllContainers";

export async function getTraefikContainers() {
    const containers = await getAllContainers();
    return containers?.filter(
        (itemTofilter) => itemTofilter.Labels["traefik.enable"] === "true" || itemTofilter.Labels["swag"] === "enable",
    );
}
