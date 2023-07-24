import { getCombinedContainerlist } from "./getCombinedContainerlist";

export async function getAllContainers() {
    const mergedContainerList = await getCombinedContainerlist();
    return mergedContainerList.map((item) => {
        const route = {
            Id: item.Id,
            Name: item.Names[0],
            Image: item.Image,
            Ports: item.Ports,
            Labels: item.Labels,
            State: item.State,
            HostConfig: item.HostConfig,
            NetworkSettings: item.NetworkSettings,
            Mounts: item.Mounts,
            serverName: item.serverName,
            serverHostname: item.serverHostname,
        };
        return route;
    });
}
