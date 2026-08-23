// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import {Test} from "forge-std/Test.sol";
import {Vm} from "forge-std/Vm.sol";
import {DeployRegistry} from "../script/DeployRegistry.s.sol";

contract DeployRegistryHarness is DeployRegistry {
    function loadConfig(address deployer) external view returns (DeploymentConfig memory) {
        return _loadConfig(deployer);
    }

    function validateMetaEvidenceUri(string memory uri) external pure {
        _validateMetaEvidenceUri(uri);
    }

    function registryFromLogs(Vm.Log[] memory entries) external pure returns (address) {
        return _registryFromLogs(entries);
    }

    function validateProductionChain() external view returns (address) {
        return _validateProductionChain();
    }
}

contract DeployRegistryTest is Test {
    DeployRegistryHarness internal harness;

    address internal constant GTCR_FACTORY = 0x794Cee5a6e1501b633eC13b8c1e327d9860FE039;
    string internal constant CANONICAL_CID = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";

    function setUp() public {
        harness = new DeployRegistryHarness();
        vm.chainId(31337);
        vm.setEnv("ALLOW_MOCK_ON_GNOSIS", "false");
    }

    function test_deploymentModesAreExplicitAndMockSafe() public {
        vm.setEnv("DEPLOYMENT_MODE", "mock");
        address deployer = makeAddr("deployer");

        DeployRegistry.DeploymentConfig memory config = harness.loadConfig(deployer);

        assertFalse(config.isProduction);
        assertEq(config.governor, deployer);
        assertEq(config.challengePeriodSeconds, 7 days);
        assertEq(config.stakeMultipliersBps[0], 10_000);
        assertEq(config.stakeMultipliersBps[1], 10_000);
        assertEq(config.stakeMultipliersBps[2], 20_000);

        vm.setEnv("DEPLOYMENT_MODE", "prod");
        vm.expectRevert("DEPLOYMENT_MODE must be exactly 'production' or 'mock'");
        harness.loadConfig(deployer);

        vm.chainId(100);
        vm.setEnv("DEPLOYMENT_MODE", "mock");
        vm.setEnv("ALLOW_MOCK_ON_GNOSIS", "false");
        vm.expectRevert("Mock mode on Gnosis requires ALLOW_MOCK_ON_GNOSIS=true");
        harness.loadConfig(deployer);
    }

    function test_metaEvidenceUriGuard() public view {
        harness.validateMetaEvidenceUri(string.concat("/ipfs/", CANONICAL_CID, "/registration.json"));
    }

    function test_metaEvidenceUriRejectsShortOrPlaceholderValues() public {
        vm.expectRevert("MetaEvidence URI must contain a canonical CIDv1");
        harness.validateMetaEvidenceUri("/ipfs/x");

        vm.expectRevert("MetaEvidence URI must not contain TODO");
        harness.validateMetaEvidenceUri(string.concat("/ipfs/", CANONICAL_CID, "/TODO.json"));
    }

    function test_registryAddressIsExtractedFromFactoryEvent() public view {
        address expected = address(0x1234567890123456789012345678901234567890);
        Vm.Log[] memory entries = new Vm.Log[](1);
        entries[0].emitter = GTCR_FACTORY;
        entries[0].topics = new bytes32[](2);
        entries[0].topics[0] = keccak256("NewGTCR(address)");
        entries[0].topics[1] = bytes32(uint256(uint160(expected)));

        assertEq(harness.registryFromLogs(entries), expected);
    }

    function test_productionPinsMatchCurrentGnosisDeployment() public {
        string memory rpcUrl = vm.envOr("GNOSIS_RPC_URL", string("https://rpc.gnosischain.com"));
        vm.createSelectFork(rpcUrl);
        DeployRegistryHarness forkHarness = new DeployRegistryHarness();

        assertEq(forkHarness.validateProductionChain(), 0x87E1bfEB31Ac4FA857a08471847122ec338F3cF2);
    }
}
