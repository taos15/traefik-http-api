import { getTraefikContainers } from "./getTraefikContainers";

export async function getFilteredServices() {
    const traefikContainers = await getTraefikContainers();
    return traefikContainers?.map((container) => {
        const keyName =
            container.Labels["traefik.service"] ??
            container.Name.replace(/^\//, "").replace(/^\w/, (c: any) => c.toUpperCase());

        const containerWebuiPort =
            container.Labels["traefik.webuiport"]?.split(",") ??
            container.Ports[container.Ports.findIndex((obj: any) => obj.IP === "0.0.0.0")]?.PublicPort;

        const serviceScheme = container.Labels["traefik.service.scheme"];

        const loadBalancerServers = [
            {
                url: `${container.serverHostname}:${containerWebuiPort}`,
            },
        ];

        return {
            [keyName]: {
                loadBalancer: {
                    servers: loadBalancerServers.map((server) =>
                        serviceScheme ? { ...server, scheme: serviceScheme } : server,
                    ),
                },
            },
        };
    });
}
