import { gertContainersList } from "./gertContainersList";

/**
 * Get a array of array of containers and flat it to ony one array
 * @returns {}
 */
export async function getCombinedContainerlist() {
    try {
        const containerList = await gertContainersList();
        if (containerList) {
            const mergedContainerList = [...containerList.flat()];
            return mergedContainerList;
        }
    } catch (error) {
        console.error((error as Error).message);
    }
}
