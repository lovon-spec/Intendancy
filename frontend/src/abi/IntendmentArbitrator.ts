// The read-only part of IntendmentArbitrator the evidence display uses, from
// lovon-spec/intendment contracts/src/IntendmentArbitrator.sol @ 582aec0
// (Case and HostProfiles.Binding layouts as compiled there).
export const intendmentArbitratorAbi = [
  {
    type: "function",
    name: "HOST",
    inputs: [],
    outputs: [{ name: "", type: "address" }],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "K",
    inputs: [],
    outputs: [{ name: "", type: "address" }],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "localDispute",
    inputs: [{ name: "remoteID", type: "uint256" }],
    outputs: [{ name: "", type: "uint256" }],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "caseOf",
    inputs: [{ name: "id", type: "uint256" }],
    outputs: [
      {
        name: "",
        type: "tuple",
        components: [
          { name: "state", type: "uint8" },
          { name: "epochId", type: "uint32" },
          { name: "choices", type: "uint256" },
          { name: "fee", type: "uint256" },
          { name: "createdAt", type: "uint256" },
          { name: "deadline", type: "uint256" },
          { name: "fundingEnd", type: "uint256" },
          { name: "earmark", type: "uint256" },
          { name: "firstForwardFailure", type: "uint256" },
          { name: "totalFunding", type: "uint256" },
          { name: "fundingUsed", type: "uint256" },
          { name: "refundableAmount", type: "uint256" },
          { name: "claimedFunding", type: "uint256" },
          { name: "kDisputeID", type: "uint256" },
          { name: "ruling", type: "uint256" },
          { name: "rulingDelivered", type: "bool" },
          { name: "insuredAtCreation", type: "bool" },
          { name: "envelopeHash", type: "bytes32" },
          {
            name: "binding",
            type: "tuple",
            components: [
              { name: "item", type: "bytes32" },
              { name: "requestIndex", type: "uint256" },
              { name: "requester", type: "address" },
              { name: "challenger", type: "address" },
              { name: "removal", type: "bool" },
              { name: "metaEvidenceID", type: "uint256" },
            ],
          },
        ],
      },
    ],
    stateMutability: "view",
  },
] as const;
