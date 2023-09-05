import { getAllContainers } from "../docker/getAllContainers";

export async function getTraefikContainers() {
    let containers = await getAllContainers();
    return containers?.filter(
        (itemTofilter) => itemTofilter.Labels["traefik.enable"] === "true" || itemTofilter.Labels["swag"] === "enable",
    );
}
