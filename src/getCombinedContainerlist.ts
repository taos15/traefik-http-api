import { gertContainersList } from "./gertContainersList";

export async function getCombinedContainerlist() {
    try {
        const containerList = await gertContainersList();
        if (containerList) {
            const mergedContainerList = [...containerList.flat()];
            return mergedContainerList;
        }
    } catch (error) {
        console.error(error);
    }
}
