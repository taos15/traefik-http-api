import { gertContainersList } from "./gertContainersList";

export async function getCombinedContainerlist() {
    const containerList = await gertContainersList();
    const mergedContainerList = [...containerList.flat()];
    return mergedContainerList;
}
