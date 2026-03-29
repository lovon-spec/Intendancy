// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title Interface for the Kleros GTCRFactory on Gnosis Chain
/// @dev Factory at 0x794Cee5a6e1501b633eC13b8c1e327d9860FE039
interface IGTCRFactory {
    event NewGTCR(address indexed _address);

    function deploy(
        address _arbitrator,
        bytes calldata _arbitratorExtraData,
        address _connectedTCR,
        string calldata _registrationMetaEvidence,
        string calldata _clearingMetaEvidence,
        address _governor,
        uint256 _submissionBaseDeposit,
        uint256 _removalBaseDeposit,
        uint256 _submissionChallengeBaseDeposit,
        uint256 _removalChallengeBaseDeposit,
        uint256 _challengePeriodDuration,
        uint256[3] calldata _stakeMultipliers
    ) external;

    function instances(uint256) external view returns (address);
    function count() external view returns (uint256);
}
