// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "forge-std/Script.sol";
import "../src/interfaces/IGTCRFactory.sol";
import "../src/interfaces/IGeneralizedTCR.sol";
import "../src/mock/MockArbitrator.sol";

/// @title Deploy Intendhub Registry
/// @dev Deploys a GeneralizedTCR via the official GTCRFactory on Gnosis Chain.
///      For testing: deploys a MockArbitrator.
///      For production: uses xKlerosLiquid at 0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002.
contract DeployRegistry is Script {
    address constant GTCR_FACTORY = 0x794Cee5a6e1501b633eC13b8c1e327d9860FE039;
    address constant XKLEROS_LIQUID = 0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002;

    function run() external {
        bool isProduction = vm.envOr("PRODUCTION", false);
        uint256 deployerKey = vm.envUint("DEPLOYER_PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);

        vm.startBroadcast(deployerKey);

        address arbitratorAddr;
        bytes memory arbitratorExtraData;

        if (isProduction) {
            arbitratorAddr = XKLEROS_LIQUID;
            // Subcourt 0 (General/Curation), 3 jurors
            arbitratorExtraData = abi.encodePacked(uint256(0), uint256(3));
        } else {
            // Deploy mock arbitrator for testing
            MockArbitrator mock = new MockArbitrator(0.001 ether, 180);
            arbitratorAddr = address(mock);
            arbitratorExtraData = bytes("");
            console.log("MockArbitrator deployed at:", arbitratorAddr);
        }

        // Meta-evidence URIs (replace with real IPFS CIDs for production)
        string memory regMetaEvidence = vm.envOr(
            "REG_META_EVIDENCE",
            string("/ipfs/TODO/registration-meta-evidence.json")
        );
        string memory clearMetaEvidence = vm.envOr(
            "CLEAR_META_EVIDENCE",
            string("/ipfs/TODO/clearing-meta-evidence.json")
        );

        // Deploy parameters
        uint256 submissionDeposit = vm.envOr("SUBMISSION_DEPOSIT", uint256(0.01 ether));
        uint256 removalDeposit = vm.envOr("REMOVAL_DEPOSIT", uint256(0.01 ether));
        uint256 submissionChallengeDeposit = vm.envOr("SUBMISSION_CHALLENGE_DEPOSIT", uint256(0.01 ether));
        uint256 removalChallengeDeposit = vm.envOr("REMOVAL_CHALLENGE_DEPOSIT", uint256(0.01 ether));
        uint256 challengePeriod = vm.envOr("CHALLENGE_PERIOD", uint256(604800)); // 7 days

        uint256[3] memory stakeMultipliers = [
            uint256(10000), // shared: 100%
            uint256(10000), // winner: 100%
            uint256(20000)  // loser: 200%
        ];

        IGTCRFactory factory = IGTCRFactory(GTCR_FACTORY);
        uint256 countBefore = factory.count();

        factory.deploy(
            arbitratorAddr,
            arbitratorExtraData,
            address(0), // connectedTCR
            regMetaEvidence,
            clearMetaEvidence,
            deployer, // governor
            submissionDeposit,
            removalDeposit,
            submissionChallengeDeposit,
            removalChallengeDeposit,
            challengePeriod,
            stakeMultipliers
        );

        address registryAddr = factory.instances(countBefore);

        vm.stopBroadcast();

        console.log("=== Intendhub Registry Deployed ===");
        console.log("Registry:       ", registryAddr);
        console.log("Arbitrator:     ", arbitratorAddr);
        console.log("Governor:       ", deployer);
        console.log("Submission dep: ", submissionDeposit);
        console.log("Challenge period:", challengePeriod, "seconds");
        console.log("Production:     ", isProduction);
    }
}
