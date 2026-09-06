function positiveInteger(value) {
  return Number.isSafeInteger(value) && value > 0;
}

function maximumUsefulDirectoryNodes(
  remainingDepth,
  branchingFactor,
  limit,
) {
  if (remainingDepth < 2 || limit <= 0) return 0;
  let nodesAtDepth = 1;
  let total = 0;
  for (let depth = 1; depth < remainingDepth && total < limit; depth += 1) {
    total = Math.min(limit, total + nodesAtDepth);
    nodesAtDepth = Math.min(limit, nodesAtDepth * branchingFactor);
  }
  return total;
}

export function minimumGenerationForestEntries({
  branchingFactor,
  maximumDirectoryNodes,
  requiredLeaves,
  slotGroups,
}) {
  if (
    !positiveInteger(branchingFactor) ||
    branchingFactor < 2 ||
    !positiveInteger(maximumDirectoryNodes) ||
    !Number.isSafeInteger(requiredLeaves) ||
    requiredLeaves < 0 ||
    !Array.isArray(slotGroups) ||
    slotGroups.some(
      (group) =>
        group === null ||
        typeof group !== "object" ||
        !positiveInteger(group.count) ||
        !positiveInteger(group.remaining_depth),
    )
  ) {
    throw new TypeError("generation forest capacity arguments were refused");
  }
  const initialSlots = slotGroups.reduce(
    (total, group) => total + group.count,
    0,
  );
  if (requiredLeaves <= initialSlots) return initialSlots;
  const requiredDirectories = Math.ceil(
    (requiredLeaves - initialSlots) / (branchingFactor - 1),
  );
  const maximumDirectories = Math.min(
    maximumDirectoryNodes,
    slotGroups.reduce(
      (total, group) =>
        Math.min(
          maximumDirectoryNodes,
          total +
            group.count *
              maximumUsefulDirectoryNodes(
                group.remaining_depth,
                branchingFactor,
                maximumDirectoryNodes,
              ),
        ),
      0,
    ),
  );
  return requiredDirectories <= maximumDirectories
    ? requiredLeaves + requiredDirectories
    : Number.POSITIVE_INFINITY;
}
