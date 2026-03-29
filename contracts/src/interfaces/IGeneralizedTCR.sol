// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./IArbitrator.sol";

/// @title Interface for Kleros GeneralizedTCR
/// @dev Covers the public functions we need for testing. Not exhaustive.
interface IGeneralizedTCR {
    enum Status { Absent, Registered, RegistrationRequested, ClearingRequested }
    enum Party { None, Requester, Challenger }

    // --- Events ---
    event ItemSubmitted(
        bytes32 indexed _itemID,
        address indexed _submitter,
        uint256 indexed _evidenceGroupID,
        bytes _data
    );
    event RequestSubmitted(
        bytes32 indexed _itemID,
        uint256 indexed _requestIndex,
        Status _requestType
    );
    event RequestEvidenceGroupID(
        bytes32 indexed _itemID,
        uint256 indexed _requestIndex,
        uint256 indexed _evidenceGroupID
    );
    event ItemStatusChange(
        bytes32 indexed _itemID,
        uint256 indexed _requestIndex,
        uint256 indexed _roundIndex,
        bool _disputed,
        bool _resolved
    );

    // --- Writes ---
    function addItem(bytes calldata _item) external payable;
    function removeItem(bytes32 _itemID, string calldata _evidence) external payable;
    function challengeRequest(bytes32 _itemID, string calldata _evidence) external payable;
    function executeRequest(bytes32 _itemID) external;
    function submitEvidence(bytes32 _itemID, string calldata _evidence) external;
    function fundAppeal(bytes32 _itemID, Party _side) external payable;

    // --- Views ---
    function getItemInfo(bytes32 _itemID)
        external view
        returns (bytes memory data, Status status, uint256 numberOfRequests);

    function getRequestInfo(bytes32 _itemID, uint256 _request)
        external view
        returns (
            bool disputed,
            uint256 disputeID,
            uint256 submissionTime,
            bool resolved,
            address payable[3] memory parties,
            uint256 numberOfRounds,
            Party ruling,
            IArbitrator requestArbitrator,
            bytes memory requestArbitratorExtraData,
            uint256 metaEvidenceID
        );

    function itemCount() external view returns (uint256 count);
    function governor() external view returns (address);
    function arbitrator() external view returns (IArbitrator);
    function arbitratorExtraData() external view returns (bytes memory);
    function submissionBaseDeposit() external view returns (uint256);
    function removalBaseDeposit() external view returns (uint256);
    function submissionChallengeBaseDeposit() external view returns (uint256);
    function removalChallengeBaseDeposit() external view returns (uint256);
    function challengePeriodDuration() external view returns (uint256);
    function itemList(uint256 _index) external view returns (bytes32);
}
