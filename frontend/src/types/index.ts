export const ItemStatus = {
  Absent: 0,
  Registered: 1,
  RegistrationRequested: 2,
  ClearingRequested: 3,
} as const;
export type ItemStatus = (typeof ItemStatus)[keyof typeof ItemStatus];

export const Party = {
  None: 0,
  Requester: 1,
  Challenger: 2,
} as const;
export type Party = (typeof Party)[keyof typeof Party];

export type DisplayStatus =
  | "registered"
  | "pending-registration"
  | "pending-removal"
  | "disputed"
  | "flagged"
  | "absent";

export interface ItemFields {
  name: string;
  sourceType: string;
  sourceLocator: string;
  category: "skill" | "plugin" | "convention";
  runtimes: string;
  description: string;
}

export interface RegistryItem {
  itemID: `0x${string}`;
  fields: ItemFields;
  rawData: `0x${string}`;
  status: ItemStatus;
  numberOfRequests: number;
  displayStatus: DisplayStatus;
}

export interface RequestInfo {
  disputed: boolean;
  disputeID: bigint;
  submissionTime: bigint;
  resolved: boolean;
  parties: readonly [`0x${string}`, `0x${string}`, `0x${string}`];
  numberOfRounds: bigint;
  ruling: number;
}

export interface RegistryParams {
  submissionBaseDeposit: bigint;
  removalBaseDeposit: bigint;
  submissionChallengeBaseDeposit: bigint;
  removalChallengeBaseDeposit: bigint;
  challengePeriodDuration: bigint;
  arbitrationCost: bigint;
}
