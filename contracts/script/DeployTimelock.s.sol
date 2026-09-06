// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";

/// @title Deploy the registry governor: a TimelockController in front of the Safe
/// @dev The registry's governor is this timelock, never the Safe directly. Every
///      governor action on the registry (deposits, challenge period, stake
///      multipliers, the arbitrator and its extra data, the MetaEvidence/policy,
///      and the governor itself) is queued publicly by the Safe and executes no
///      earlier than the delay; anyone may execute a matured operation; the Safe
///      may cancel; the timelock administers itself. Consumers pin this address
///      and the policy versions it announces (verified-snapshot-spec §3, §11).
///
///      Environment:
///        DEPLOYER_PRIVATE_KEY        the deployer (pays gas; holds no role afterwards)
///        TIMELOCK_PROPOSER           the Safe: proposer and canceller
///        TIMELOCK_MIN_DELAY_SECONDS  at least 7 days in production (604800)
///        ALLOW_SHORT_TIMELOCK=true   tests only: permit a shorter delay
///        ALLOW_EOA_PROPOSER=true     tests only: permit a proposer without code
contract DeployTimelock is Script {
    uint256 public constant MIN_DELAY_SECONDS = 7 days;

    function run() external returns (TimelockController timelock) {
        uint256 deployerKey = vm.envUint("DEPLOYER_PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        address proposer = vm.envAddress("TIMELOCK_PROPOSER");
        uint256 delay = vm.envUint("TIMELOCK_MIN_DELAY_SECONDS");

        require(proposer != address(0), "TIMELOCK_PROPOSER must not be zero");
        require(
            proposer.code.length > 0 || vm.envOr("ALLOW_EOA_PROPOSER", false),
            "TIMELOCK_PROPOSER must be a contract (the Safe); ALLOW_EOA_PROPOSER=true is for tests only"
        );
        require(
            delay >= MIN_DELAY_SECONDS || vm.envOr("ALLOW_SHORT_TIMELOCK", false),
            "TIMELOCK_MIN_DELAY_SECONDS must be at least 7 days; ALLOW_SHORT_TIMELOCK=true is for tests only"
        );

        address[] memory proposers = new address[](1);
        proposers[0] = proposer;
        address[] memory executors = new address[](1);
        executors[0] = address(0); // open execution: anyone may execute a matured operation

        vm.startBroadcast(deployerKey);
        timelock = new TimelockController(delay, proposers, executors, address(0));
        vm.stopBroadcast();

        // Post-conditions, read back from the deployed state.
        require(timelock.getMinDelay() == delay, "timelock delay mismatch");
        require(timelock.hasRole(timelock.PROPOSER_ROLE(), proposer), "proposer role missing");
        require(timelock.hasRole(timelock.CANCELLER_ROLE(), proposer), "canceller role missing");
        require(timelock.hasRole(timelock.EXECUTOR_ROLE(), address(0)), "execution must be open");
        require(timelock.hasRole(timelock.DEFAULT_ADMIN_ROLE(), address(timelock)), "timelock must administer itself");
        require(!timelock.hasRole(timelock.DEFAULT_ADMIN_ROLE(), proposer), "proposer must not administer the timelock");
        require(!timelock.hasRole(timelock.DEFAULT_ADMIN_ROLE(), deployer), "deployer must not administer the timelock");

        console.log("TimelockController:", address(timelock));
        console.log("Min delay (seconds):", delay);
        console.log("Proposer and canceller (the Safe):", proposer);
        console.log("Deployer (no role):", deployer);
        console.log("Runtime code hash:");
        console.logBytes32(address(timelock).codehash);
        console.log("Next: verify with tools/launch/verify-timelock.sh, then set GOVERNOR to this address and GOVERNOR_PROPOSER to the Safe for DeployRegistry.");
    }
}
