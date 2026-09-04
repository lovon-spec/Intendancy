// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import {Script} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
import {Vm} from "forge-std/Vm.sol";
import {IGTCRFactory} from "../src/interfaces/IGTCRFactory.sol";
import {MockArbitrator} from "../src/mock/MockArbitrator.sol";

/// @title Deploy Intendancy Registry
/// @dev Deploys an unmodified GeneralizedTCR via the official GTCRFactory.
///      Production mode is deliberately fail-closed and Gnosis-specific.
contract DeployRegistry is Script {
    address constant GTCR_FACTORY = 0x794Cee5a6e1501b633eC13b8c1e327d9860FE039;
    address constant XKLEROS_LIQUID = 0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002;

    bytes32 constant GTCR_FACTORY_CODEHASH = 0xf0fee12644a2e3e5235ffea25a4946101ebdcebd45be1393b142e0ba1420395c;
    bytes32 constant XKLEROS_LIQUID_CODEHASH = 0x91016740a8855260560e1e8d22bef33119ca86d5524509a1b2ae0c783e1e44e0;
    bytes32 constant EIP1967_IMPLEMENTATION_SLOT = 0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc;
    address constant XKLEROS_LIQUID_IMPLEMENTATION = 0x87E1bfEB31Ac4FA857a08471847122ec338F3cF2;
    bytes32 constant XKLEROS_LIQUID_IMPLEMENTATION_CODEHASH =
        0xff3724552a8b219e6b5683ad192e668dfd9b0902c0e1c21560e6cafb7110d941;
    bytes32 constant NEW_GTCR_EVENT_SIGNATURE = keccak256("NewGTCR(address)");

    uint256 constant GNOSIS_CHAIN_ID = 100;
    uint256 constant CURATION_COURT_ID = 19;

    struct DeploymentConfig {
        bool isProduction;
        address governor;
        address arbitrator;
        address arbitratorImplementation;
        bytes arbitratorExtraData;
        uint256 courtId;
        uint256 minJurors;
        string registrationMetaEvidence;
        string clearingMetaEvidence;
        uint256 submissionBaseDepositWei;
        uint256 removalBaseDepositWei;
        uint256 submissionChallengeBaseDepositWei;
        uint256 removalChallengeBaseDepositWei;
        uint256 challengePeriodSeconds;
        uint256[3] stakeMultipliersBps;
    }

    function run() external {
        uint256 deployerKey = vm.envUint("DEPLOYER_PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        DeploymentConfig memory config = _loadConfig(deployer);

        vm.startBroadcast(deployerKey);

        if (!config.isProduction) {
            MockArbitrator mock = new MockArbitrator(0.001 ether, 180);
            config.arbitrator = address(mock);
            config.arbitratorExtraData = bytes("");
            console.log("MockArbitrator deployed at:", config.arbitrator);
        }

        vm.recordLogs();
        IGTCRFactory(GTCR_FACTORY)
            .deploy(
                config.arbitrator,
                config.arbitratorExtraData,
                address(0), // connectedTCR
                config.registrationMetaEvidence,
                config.clearingMetaEvidence,
                config.governor,
                config.submissionBaseDepositWei,
                config.removalBaseDepositWei,
                config.submissionChallengeBaseDepositWei,
                config.removalChallengeBaseDepositWei,
                config.challengePeriodSeconds,
                config.stakeMultipliersBps
            );
        address predictedRegistry = _registryFromLogs(vm.getRecordedLogs());

        vm.stopBroadcast();

        console.log("=== Intendancy Registry Deployed ===");
        console.log("Predicted registry (verify mined NewGTCR receipt):", predictedRegistry);
        console.log("Arbitrator:", config.arbitrator);
        console.log("Governor:", config.governor);
        if (config.isProduction) {
            console.log("Arbitrator implementation:", config.arbitratorImplementation);
            console.log("Curation court ID:", config.courtId);
            console.log("Minimum jurors:", config.minJurors);
        }
        console.log("Submission base deposit (wei):", config.submissionBaseDepositWei);
        console.log("Removal base deposit (wei):", config.removalBaseDepositWei);
        console.log("Submission challenge base deposit (wei):", config.submissionChallengeBaseDepositWei);
        console.log("Removal challenge base deposit (wei):", config.removalChallengeBaseDepositWei);
        console.log("Challenge period (seconds):", config.challengePeriodSeconds);
        console.log("Shared stake multiplier (bps):", config.stakeMultipliersBps[0]);
        console.log("Winner stake multiplier (bps):", config.stakeMultipliersBps[1]);
        console.log("Loser stake multiplier (bps):", config.stakeMultipliersBps[2]);
        console.log("Registration MetaEvidence:", config.registrationMetaEvidence);
        console.log("Clearing MetaEvidence:", config.clearingMetaEvidence);
        console.log("Production:", config.isProduction);
    }

    function _loadConfig(address deployer) internal view returns (DeploymentConfig memory config) {
        string memory deploymentMode = vm.envString("DEPLOYMENT_MODE");
        bytes32 modeHash = keccak256(bytes(deploymentMode));

        if (modeHash == keccak256("production")) {
            config.isProduction = true;
            config.arbitratorImplementation = _validateProductionChain();
            config.governor = vm.envAddress("GOVERNOR");
            require(config.governor != address(0), "GOVERNOR must not be zero");

            config.courtId = CURATION_COURT_ID;
            config.minJurors = vm.envUint("MIN_JURORS");
            require(config.minJurors > 0, "MIN_JURORS must be greater than zero");
            config.arbitrator = XKLEROS_LIQUID;
            config.arbitratorExtraData = abi.encodePacked(config.courtId, config.minJurors);

            config.registrationMetaEvidence = vm.envString("REG_META_EVIDENCE");
            config.clearingMetaEvidence = vm.envString("CLEAR_META_EVIDENCE");
            _validateMetaEvidenceUri(config.registrationMetaEvidence);
            _validateMetaEvidenceUri(config.clearingMetaEvidence);
            require(
                keccak256(bytes(config.registrationMetaEvidence)) != keccak256(bytes(config.clearingMetaEvidence)),
                "MetaEvidence URIs must differ"
            );

            config.submissionBaseDepositWei = vm.envUint("SUBMISSION_BASE_DEPOSIT_WEI");
            config.removalBaseDepositWei = vm.envUint("REMOVAL_BASE_DEPOSIT_WEI");
            config.submissionChallengeBaseDepositWei = vm.envUint("SUBMISSION_CHALLENGE_BASE_DEPOSIT_WEI");
            config.removalChallengeBaseDepositWei = vm.envUint("REMOVAL_CHALLENGE_BASE_DEPOSIT_WEI");
            config.challengePeriodSeconds = vm.envUint("CHALLENGE_PERIOD_SECONDS");
            config.stakeMultipliersBps = [
                vm.envUint("SHARED_STAKE_MULTIPLIER_BPS"),
                vm.envUint("WINNER_STAKE_MULTIPLIER_BPS"),
                vm.envUint("LOSER_STAKE_MULTIPLIER_BPS")
            ];
        } else if (modeHash == keccak256("mock")) {
            config.isProduction = false;
            require(
                block.chainid != GNOSIS_CHAIN_ID || vm.envOr("ALLOW_MOCK_ON_GNOSIS", false),
                "Mock mode on Gnosis requires ALLOW_MOCK_ON_GNOSIS=true"
            );
            config.governor = deployer;
            config.registrationMetaEvidence =
                vm.envOr("REG_META_EVIDENCE", string("/ipfs/TODO/registration-meta-evidence.json"));
            config.clearingMetaEvidence =
                vm.envOr("CLEAR_META_EVIDENCE", string("/ipfs/TODO/clearing-meta-evidence.json"));
            config.submissionBaseDepositWei = vm.envOr("SUBMISSION_BASE_DEPOSIT_WEI", uint256(0.01 ether));
            config.removalBaseDepositWei = vm.envOr("REMOVAL_BASE_DEPOSIT_WEI", uint256(0.01 ether));
            config.submissionChallengeBaseDepositWei =
                vm.envOr("SUBMISSION_CHALLENGE_BASE_DEPOSIT_WEI", uint256(0.01 ether));
            config.removalChallengeBaseDepositWei = vm.envOr("REMOVAL_CHALLENGE_BASE_DEPOSIT_WEI", uint256(0.01 ether));
            config.challengePeriodSeconds = vm.envOr("CHALLENGE_PERIOD_SECONDS", uint256(604800));
            config.stakeMultipliersBps = [uint256(10000), uint256(10000), uint256(20000)];
        } else {
            revert("DEPLOYMENT_MODE must be exactly 'production' or 'mock'");
        }

        require(config.challengePeriodSeconds > 0, "Challenge period must be greater than zero");
    }

    function _validateProductionChain() internal view returns (address implementation) {
        require(block.chainid == GNOSIS_CHAIN_ID, "Production deployment requires Gnosis Chain (100)");
        require(GTCR_FACTORY.codehash == GTCR_FACTORY_CODEHASH, "Unexpected GTCRFactory runtime codehash");
        require(XKLEROS_LIQUID.codehash == XKLEROS_LIQUID_CODEHASH, "Unexpected arbitrator runtime codehash");
        implementation = address(uint160(uint256(vm.load(XKLEROS_LIQUID, EIP1967_IMPLEMENTATION_SLOT))));
        require(implementation == XKLEROS_LIQUID_IMPLEMENTATION, "Unexpected arbitrator implementation address");
        require(
            implementation.codehash == XKLEROS_LIQUID_IMPLEMENTATION_CODEHASH,
            "Unexpected arbitrator implementation codehash"
        );
    }

    function _validateMetaEvidenceUri(string memory uri) internal pure {
        bytes memory value = bytes(uri);
        bytes memory prefix = bytes("/ipfs/");
        uint256 cidLength = 59; // CIDv1 base32 with a one-byte codec and sha2-256 multihash.
        require(value.length >= prefix.length + cidLength, "MetaEvidence URI must contain a canonical CIDv1");
        for (uint256 i = 0; i < prefix.length; i++) {
            require(value[i] == prefix[i], "MetaEvidence URI must start with /ipfs/");
        }
        require(value[prefix.length] == bytes1("b"), "MetaEvidence CID must use lowercase base32");
        for (uint256 i = prefix.length + 1; i < prefix.length + cidLength; i++) {
            bytes1 character = value[i];
            bool isLowercaseLetter = character >= bytes1("a") && character <= bytes1("z");
            bool isBase32Digit = character >= bytes1("2") && character <= bytes1("7");
            require(isLowercaseLetter || isBase32Digit, "MetaEvidence CID is not canonical base32");
        }
        if (value.length > prefix.length + cidLength) {
            require(value[prefix.length + cidLength] == bytes1("/"), "MetaEvidence CID has an invalid suffix");
            require(value.length > prefix.length + cidLength + 1, "MetaEvidence path must not be empty");
        }
        require(!_containsTodo(value), "MetaEvidence URI must not contain TODO");
    }

    function _containsTodo(bytes memory value) internal pure returns (bool) {
        bytes memory needle = bytes("TODO");
        if (value.length < needle.length) return false;
        for (uint256 i = 0; i <= value.length - needle.length; i++) {
            bool matched = true;
            for (uint256 j = 0; j < needle.length; j++) {
                if (value[i + j] != needle[j]) {
                    matched = false;
                    break;
                }
            }
            if (matched) return true;
        }
        return false;
    }

    function _registryFromLogs(Vm.Log[] memory entries) internal pure returns (address registry) {
        for (uint256 i = 0; i < entries.length; i++) {
            if (
                entries[i].emitter == GTCR_FACTORY && entries[i].topics.length == 2
                    && entries[i].topics[0] == NEW_GTCR_EVENT_SIGNATURE
            ) {
                require(registry == address(0), "Multiple NewGTCR events emitted");
                registry = address(uint160(uint256(entries[i].topics[1])));
            }
        }
        require(registry != address(0), "GTCRFactory did not emit NewGTCR");
    }
}
