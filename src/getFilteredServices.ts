import { getTraefikContainers } from "./getTraefikContainers";

export async function getFilteredServices() {
    const traefikContainers = await getTraefikContainers();
    return traefikContainers?.map((container) => {
        const keyName = container.Name.replace(/^\//, "").replace(/^\w/, (c: any) => c.toUpperCase());
        const containerWebuiPort =
            container.Labels["traefik.webuiport"]?.split(",") ??
            container.Ports[container.Ports.findIndex((obj: any) => obj.IP === "0.0.0.0")]?.PublicPort;
        return {
            [keyName]: {
                loadBalancer: {
                    servers: [
                        {
                            url: `${container.serverHostname}:${containerWebuiPort}`,
                        },
                    ],
                },
            },
        };
    });
}
