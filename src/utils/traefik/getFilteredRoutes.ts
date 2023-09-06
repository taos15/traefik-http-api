import { domain } from "../../app";
import { getTraefikContainers } from "./getTraefikContainers";

export async function getFilteredRoutes() {
    let traefikContainers = await getTraefikContainers();
    return traefikContainers?.map((container) => {
        const keyName =
            container.Labels["traefik.name"] ??
            container.Name.replace(/^\//, "").replace(/^\w/, (c: any) => c.toUpperCase());

        const constainerHostname = container.Labels["traefik.hostname"] ?? keyName;

        const routerService = container.Labels["traefik.service"] ?? keyName;

        const containerMiddlewares: string[] =
            container.Labels["traefik.middlewares"]?.split(",") ??
            (container.Labels["authelia_auth"] === "false" ? [] : ["auth"]);

        const containerEntrypoints = container.Labels["traefik.entrypoints"]?.split(",") ?? ["https"];

        return {
            [keyName]: {
                entryPoints: containerEntrypoints,
                rule: `Host(` + `\`` + constainerHostname + `.${domain}\`)`,
                service: routerService,
                middlewares: containerMiddlewares,
            },
        };
    });
}
