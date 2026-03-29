// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "forge-std/Test.sol";
import "../src/interfaces/IGTCRFactory.sol";
import "../src/interfaces/IGeneralizedTCR.sol";
import "../src/interfaces/IArbitrator.sol";
import "../src/mock/MockArbitrator.sol";

/// @title Intendhub Registry Integration Tests
/// @dev Tests deploy a GeneralizedTCR via the real GTCRFactory on a Gnosis fork,
///      using a MockArbitrator for controllable dispute resolution.
contract IntendhubRegistryTest is Test {
    // --- Gnosis Chain addresses ---
    address constant GTCR_FACTORY = 0x794Cee5a6e1501b633eC13b8c1e327d9860FE039;

    // --- Test actors ---
    address governor;
    address submitter;
    address challenger;
    address guardian;

    // --- Contracts ---
    MockArbitrator arbitrator;
    IGeneralizedTCR registry;

    // --- Parameters ---
    uint256 constant ARBITRATION_PRICE = 0.001 ether;
    uint256 constant APPEAL_TIMEOUT = 180; // seconds
    uint256 constant SUBMISSION_DEPOSIT = 0.01 ether;
    uint256 constant REMOVAL_DEPOSIT = 0.01 ether;
    uint256 constant SUBMISSION_CHALLENGE_DEPOSIT = 0.01 ether;
    uint256 constant REMOVAL_CHALLENGE_DEPOSIT = 0.01 ether;
    uint256 constant CHALLENGE_PERIOD = 302400; // 3.5 days

    // Stake multipliers: shared=100%, winner=100%, loser=200% (basis points)
    uint256[3] stakeMultipliers = [uint256(10000), uint256(10000), uint256(20000)];

    // --- Test item data (raw bytes, encoding is a frontend concern) ---
    // Represents: Name="test-skill", SourceType="git",
    // SourceLocator="https://github.com/test/repo@abc123", Category="skill",
    // Runtimes="claude_code,generic", Description="A test skill"
    bytes constant TEST_ITEM_1 = abi.encodePacked(
        "test-skill",
        "|git",
        "|https://github.com/test/repo@abc1230000000000000000000000000000000000",
        "|skill",
        "|claude_code,generic",
        "|A test skill for integration testing"
    );

    bytes constant TEST_ITEM_2 = abi.encodePacked(
        "another-skill",
        "|git",
        "|https://github.com/other/repo@def4560000000000000000000000000000000000",
        "|convention",
        "|cursor,generic",
        "|A convention file for testing"
    );

    // --- Events we check ---
    // Note: we use the raw event signatures since the contract is 0.5.x compiled
    event ItemSubmitted(
        bytes32 indexed _itemID,
        address indexed _submitter,
        uint256 indexed _evidenceGroupID,
        bytes _data
    );
    event ItemStatusChange(
        bytes32 indexed _itemID,
        uint256 indexed _requestIndex,
        uint256 indexed _roundIndex,
        bool _disputed,
        bool _resolved
    );

    function setUp() public {
        // Fork Gnosis Chain
        string memory rpcUrl = vm.envOr("GNOSIS_RPC_URL", string("https://rpc.gnosischain.com"));
        vm.createSelectFork(rpcUrl);

        // Set up actors
        governor = makeAddr("governor");
        submitter = makeAddr("submitter");
        challenger = makeAddr("challenger");
        guardian = makeAddr("guardian");

        // Fund actors
        vm.deal(governor, 100 ether);
        vm.deal(submitter, 100 ether);
        vm.deal(challenger, 100 ether);
        vm.deal(guardian, 100 ether);

        // Deploy mock arbitrator
        vm.prank(governor);
        arbitrator = new MockArbitrator(ARBITRATION_PRICE, APPEAL_TIMEOUT);

        // Deploy registry via the real GTCRFactory
        IGTCRFactory factory = IGTCRFactory(GTCR_FACTORY);
        uint256 countBefore = factory.count();

        vm.prank(governor);
        factory.deploy(
            address(arbitrator),
            bytes(""),  // arbitratorExtraData (mock ignores it)
            address(0), // connectedTCR (none for V1)
            "/ipfs/QmTest/registration-meta-evidence.json",
            "/ipfs/QmTest/clearing-meta-evidence.json",
            governor,
            SUBMISSION_DEPOSIT,
            REMOVAL_DEPOSIT,
            SUBMISSION_CHALLENGE_DEPOSIT,
            REMOVAL_CHALLENGE_DEPOSIT,
            CHALLENGE_PERIOD,
            stakeMultipliers
        );

        // Get the deployed registry address
        uint256 countAfter = factory.count();
        assertEq(countAfter, countBefore + 1, "Factory should have one more instance");
        address registryAddr = factory.instances(countAfter - 1);
        registry = IGeneralizedTCR(registryAddr);
    }

    // ========================================================================
    // Deployment tests
    // ========================================================================

    function test_deployViaFactory() public view {
        assertEq(registry.governor(), governor);
        assertEq(address(registry.arbitrator()), address(arbitrator));
        assertEq(registry.submissionBaseDeposit(), SUBMISSION_DEPOSIT);
        assertEq(registry.removalBaseDeposit(), REMOVAL_DEPOSIT);
        assertEq(registry.submissionChallengeBaseDeposit(), SUBMISSION_CHALLENGE_DEPOSIT);
        assertEq(registry.removalChallengeBaseDeposit(), REMOVAL_CHALLENGE_DEPOSIT);
        assertEq(registry.challengePeriodDuration(), CHALLENGE_PERIOD);
    }

    // ========================================================================
    // Submission tests
    // ========================================================================

    function test_submitItem() public {
        bytes32 expectedItemID = keccak256(TEST_ITEM_1);
        uint256 totalDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;

        vm.prank(submitter);
        registry.addItem{value: totalDeposit}(TEST_ITEM_1);

        // Check item status
        (, IGeneralizedTCR.Status status, uint256 numRequests) = registry.getItemInfo(expectedItemID);
        assertEq(uint256(status), uint256(IGeneralizedTCR.Status.RegistrationRequested));
        assertEq(numRequests, 1);
    }

    function test_submitItemIDConsistency() public {
        bytes32 expectedItemID = keccak256(TEST_ITEM_1);
        uint256 totalDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;

        vm.prank(submitter);
        registry.addItem{value: totalDeposit}(TEST_ITEM_1);

        // The first item in the list should match our expected ID
        bytes32 actualItemID = registry.itemList(0);
        assertEq(actualItemID, expectedItemID);
    }

    function test_submitItemRefundsExcess() public {
        uint256 totalDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;
        uint256 overpayment = 1 ether;
        uint256 balanceBefore = submitter.balance;

        vm.prank(submitter);
        registry.addItem{value: totalDeposit + overpayment}(TEST_ITEM_1);

        // Submitter should have been refunded the overpayment
        uint256 balanceAfter = submitter.balance;
        assertEq(balanceBefore - balanceAfter, totalDeposit, "Should only deduct exact deposit");
    }

    function test_submitItemInsufficientDeposit() public {
        vm.prank(submitter);
        vm.expectRevert();
        registry.addItem{value: 0.001 ether}(TEST_ITEM_1);
    }

    function test_submitDuplicateReverts() public {
        uint256 totalDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;

        vm.prank(submitter);
        registry.addItem{value: totalDeposit}(TEST_ITEM_1);

        // Submitting the same item again should revert
        vm.prank(submitter);
        vm.expectRevert();
        registry.addItem{value: totalDeposit}(TEST_ITEM_1);
    }

    // ========================================================================
    // Execution tests (unchallenged)
    // ========================================================================

    function test_executeUnchallengedSubmission() public {
        bytes32 itemID = keccak256(TEST_ITEM_1);
        uint256 totalDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;

        vm.prank(submitter);
        registry.addItem{value: totalDeposit}(TEST_ITEM_1);

        // Advance time past challenge period
        vm.warp(block.timestamp + CHALLENGE_PERIOD + 1);

        // Execute
        registry.executeRequest(itemID);

        // Check item is now Registered
        (, IGeneralizedTCR.Status status,) = registry.getItemInfo(itemID);
        assertEq(uint256(status), uint256(IGeneralizedTCR.Status.Registered));
    }

    function test_executeBeforePeriodReverts() public {
        bytes32 itemID = keccak256(TEST_ITEM_1);
        uint256 totalDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;

        vm.prank(submitter);
        registry.addItem{value: totalDeposit}(TEST_ITEM_1);

        // Try to execute immediately — should revert
        vm.expectRevert();
        registry.executeRequest(itemID);
    }

    // ========================================================================
    // Challenge tests
    // ========================================================================

    function test_challengeSubmission() public {
        bytes32 itemID = keccak256(TEST_ITEM_1);
        uint256 submitDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;
        uint256 challengeDeposit = SUBMISSION_CHALLENGE_DEPOSIT + ARBITRATION_PRICE;

        // Submit
        vm.prank(submitter);
        registry.addItem{value: submitDeposit}(TEST_ITEM_1);

        // Challenge
        vm.prank(challenger);
        registry.challengeRequest{value: challengeDeposit}(
            itemID,
            "/ipfs/QmEvidence/challenge-evidence.json"
        );

        // Check request is now disputed
        (bool disputed,,,,,,,,,) = registry.getRequestInfo(itemID, 0);
        assertTrue(disputed, "Request should be disputed");

        // Arbitrator should have one dispute
        assertEq(arbitrator.disputeCount(), 1);
    }

    function test_challengeAfterPeriodReverts() public {
        bytes32 itemID = keccak256(TEST_ITEM_1);
        uint256 submitDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;
        uint256 challengeDeposit = SUBMISSION_CHALLENGE_DEPOSIT + ARBITRATION_PRICE;

        vm.prank(submitter);
        registry.addItem{value: submitDeposit}(TEST_ITEM_1);

        // Advance past challenge period
        vm.warp(block.timestamp + CHALLENGE_PERIOD + 1);

        // Challenge should revert
        vm.prank(challenger);
        vm.expectRevert();
        registry.challengeRequest{value: challengeDeposit}(itemID, "");
    }

    // ========================================================================
    // Dispute resolution tests
    // ========================================================================

    function test_rulingAcceptsSubmission() public {
        bytes32 itemID = keccak256(TEST_ITEM_1);
        uint256 submitDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;
        uint256 challengeDeposit = SUBMISSION_CHALLENGE_DEPOSIT + ARBITRATION_PRICE;

        // Submit and challenge
        vm.prank(submitter);
        registry.addItem{value: submitDeposit}(TEST_ITEM_1);
        vm.prank(challenger);
        registry.challengeRequest{value: challengeDeposit}(itemID, "");

        // Rule in favor of requester (1 = Requester wins = item gets registered)
        vm.prank(governor);
        arbitrator.giveRuling(0, 1);

        // Advance past appeal period
        vm.warp(block.timestamp + APPEAL_TIMEOUT + 1);
        arbitrator.executeRuling(0);

        // Item should be Registered
        (, IGeneralizedTCR.Status status,) = registry.getItemInfo(itemID);
        assertEq(uint256(status), uint256(IGeneralizedTCR.Status.Registered));
    }

    function test_rulingRejectsSubmission() public {
        bytes32 itemID = keccak256(TEST_ITEM_1);
        uint256 submitDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;
        uint256 challengeDeposit = SUBMISSION_CHALLENGE_DEPOSIT + ARBITRATION_PRICE;

        // Submit and challenge
        vm.prank(submitter);
        registry.addItem{value: submitDeposit}(TEST_ITEM_1);
        vm.prank(challenger);
        registry.challengeRequest{value: challengeDeposit}(itemID, "");

        // Rule in favor of challenger (2 = Challenger wins = item rejected)
        vm.prank(governor);
        arbitrator.giveRuling(0, 2);

        // Advance past appeal period
        vm.warp(block.timestamp + APPEAL_TIMEOUT + 1);
        arbitrator.executeRuling(0);

        // Item should be Absent (rejected)
        (, IGeneralizedTCR.Status status,) = registry.getItemInfo(itemID);
        assertEq(uint256(status), uint256(IGeneralizedTCR.Status.Absent));
    }

    // ========================================================================
    // Guardian removal pattern tests
    // ========================================================================

    function test_guardianRemovalRequest() public {
        bytes32 itemID = keccak256(TEST_ITEM_1);
        uint256 submitDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;
        uint256 removalDeposit = REMOVAL_DEPOSIT + ARBITRATION_PRICE;

        // Submit and register the item
        vm.prank(submitter);
        registry.addItem{value: submitDeposit}(TEST_ITEM_1);
        vm.warp(block.timestamp + CHALLENGE_PERIOD + 1);
        registry.executeRequest(itemID);

        // Verify it's registered
        (, IGeneralizedTCR.Status status,) = registry.getItemInfo(itemID);
        assertEq(uint256(status), uint256(IGeneralizedTCR.Status.Registered));

        // Guardian requests removal
        vm.prank(guardian);
        registry.removeItem{value: removalDeposit}(
            itemID,
            "/ipfs/QmEvidence/guardian-removal-evidence.json"
        );

        // Item should be in ClearingRequested status
        (, status,) = registry.getItemInfo(itemID);
        assertEq(uint256(status), uint256(IGeneralizedTCR.Status.ClearingRequested));

        // Verify the requester is the guardian address
        (,,,, address payable[3] memory parties,,,,, ) = registry.getRequestInfo(itemID, 1);
        assertEq(parties[1], guardian, "Removal requester should be guardian");
    }

    function test_guardianRemovalExecutes() public {
        bytes32 itemID = keccak256(TEST_ITEM_1);
        uint256 submitDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;
        uint256 removalDeposit = REMOVAL_DEPOSIT + ARBITRATION_PRICE;

        // Submit → register
        vm.prank(submitter);
        registry.addItem{value: submitDeposit}(TEST_ITEM_1);
        uint256 t1 = block.timestamp + CHALLENGE_PERIOD + 1;
        vm.warp(t1);
        registry.executeRequest(itemID);

        // Guardian requests removal
        vm.prank(guardian);
        registry.removeItem{value: removalDeposit}(itemID, "");

        // Advance past challenge period (no one challenges the guardian)
        vm.warp(t1 + CHALLENGE_PERIOD + 1);
        registry.executeRequest(itemID);

        // Item should be Absent (removed)
        (, IGeneralizedTCR.Status status,) = registry.getItemInfo(itemID);
        assertEq(uint256(status), uint256(IGeneralizedTCR.Status.Absent));
    }

    function test_guardianRemovalCanBeChallenged() public {
        bytes32 itemID = keccak256(TEST_ITEM_1);
        uint256 submitDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;
        uint256 removalDeposit = REMOVAL_DEPOSIT + ARBITRATION_PRICE;
        uint256 challengeDeposit = REMOVAL_CHALLENGE_DEPOSIT + ARBITRATION_PRICE;

        // Submit → register
        vm.prank(submitter);
        registry.addItem{value: submitDeposit}(TEST_ITEM_1);
        vm.warp(block.timestamp + CHALLENGE_PERIOD + 1);
        registry.executeRequest(itemID);

        // Guardian requests removal
        vm.prank(guardian);
        registry.removeItem{value: removalDeposit}(itemID, "");

        // Someone challenges the guardian's removal
        vm.prank(challenger);
        registry.challengeRequest{value: challengeDeposit}(itemID, "Guardian removal unjustified");

        // Rule against the guardian (2 = Challenger wins, item stays)
        vm.prank(governor);
        arbitrator.giveRuling(0, 2);
        vm.warp(block.timestamp + APPEAL_TIMEOUT + 1);
        arbitrator.executeRuling(0);

        // Item should still be Registered (guardian's removal rejected)
        (, IGeneralizedTCR.Status status,) = registry.getItemInfo(itemID);
        assertEq(uint256(status), uint256(IGeneralizedTCR.Status.Registered));
    }

    // ========================================================================
    // Multiple items test
    // ========================================================================

    function test_multipleItems() public {
        uint256 totalDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;

        // Submit two different items
        vm.prank(submitter);
        registry.addItem{value: totalDeposit}(TEST_ITEM_1);
        vm.prank(submitter);
        registry.addItem{value: totalDeposit}(TEST_ITEM_2);

        // Both should be RegistrationRequested
        (, IGeneralizedTCR.Status s1,) = registry.getItemInfo(keccak256(TEST_ITEM_1));
        (, IGeneralizedTCR.Status s2,) = registry.getItemInfo(keccak256(TEST_ITEM_2));
        assertEq(uint256(s1), uint256(IGeneralizedTCR.Status.RegistrationRequested));
        assertEq(uint256(s2), uint256(IGeneralizedTCR.Status.RegistrationRequested));

        assertEq(registry.itemCount(), 2);
    }

    // ========================================================================
    // Evidence submission test
    // ========================================================================

    function test_submitEvidence() public {
        bytes32 itemID = keccak256(TEST_ITEM_1);
        uint256 totalDeposit = SUBMISSION_DEPOSIT + ARBITRATION_PRICE;

        vm.prank(submitter);
        registry.addItem{value: totalDeposit}(TEST_ITEM_1);

        // Anyone can submit evidence
        vm.prank(submitter);
        registry.submitEvidence(itemID, "/ipfs/QmEvidence/support.json");
    }
}
