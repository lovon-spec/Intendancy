import type { RegistryParams } from "../types";
import { ItemStatus } from "../types";

export function submissionDeposit(params: RegistryParams): bigint {
  return params.submissionBaseDeposit + params.arbitrationCost;
}

export function removalDeposit(params: RegistryParams): bigint {
  return params.removalBaseDeposit + params.arbitrationCost;
}

export function challengeDeposit(params: RegistryParams, itemStatus: ItemStatus): bigint {
  const base =
    itemStatus === ItemStatus.RegistrationRequested
      ? params.submissionChallengeBaseDeposit
      : params.removalChallengeBaseDeposit;
  return base + params.arbitrationCost;
}
