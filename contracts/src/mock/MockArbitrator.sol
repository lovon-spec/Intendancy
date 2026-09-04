// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "../interfaces/IArbitrator.sol";
import "../interfaces/IArbitrable.sol";

/// @title MockArbitrator - Controllable arbitrator for testing
/// @dev Implements ERC-792 IArbitrator. Owner can give rulings directly.
///      Supports appeal periods so GeneralizedTCR's fundAppeal() works.
contract MockArbitrator is IArbitrator {
    address public owner;
    uint256 public arbitrationPrice;
    uint256 public appealTimeout;

    uint256 constant NOT_PAYABLE_VALUE = type(uint256).max / 2;

    struct Dispute {
        IArbitrable arbitrated;
        uint256 choices;
        uint256 fee;
        uint256 ruling;
        DisputeStatus status;
        uint256 appealStart;
    }

    Dispute[] public disputes;

    modifier onlyOwner() {
        require(msg.sender == owner, "Only owner");
        _;
    }

    constructor(uint256 _arbitrationPrice, uint256 _appealTimeout) {
        owner = msg.sender;
        arbitrationPrice = _arbitrationPrice;
        appealTimeout = _appealTimeout;
    }

    function arbitrationCost(bytes calldata) external view override returns (uint256 cost) {
        return arbitrationPrice;
    }

    function createDispute(uint256 _choices, bytes calldata) external payable override returns (uint256 disputeID) {
        require(msg.value >= arbitrationPrice, "Insufficient arbitration fee");
        disputeID = disputes.length;
        disputes.push(
            Dispute({
                arbitrated: IArbitrable(msg.sender),
                choices: _choices,
                fee: msg.value,
                ruling: 0,
                status: DisputeStatus.Waiting,
                appealStart: 0
            })
        );
        emit DisputeCreation(disputeID, IArbitrable(msg.sender));
    }

    /// @dev Give a provisional ruling, opening the appeal period.
    function giveRuling(uint256 _disputeID, uint256 _ruling) external onlyOwner {
        Dispute storage d = disputes[_disputeID];
        require(d.status == DisputeStatus.Waiting, "Dispute not waiting");
        require(_ruling <= d.choices, "Invalid ruling");
        d.ruling = _ruling;
        d.status = DisputeStatus.Appealable;
        d.appealStart = block.timestamp;
        emit AppealPossible(_disputeID, d.arbitrated);
    }

    /// @dev Execute the ruling after appeal period expires.
    function executeRuling(uint256 _disputeID) external {
        Dispute storage d = disputes[_disputeID];
        require(d.status == DisputeStatus.Appealable, "Not appealable");
        require(block.timestamp >= d.appealStart + appealTimeout, "Appeal period not over");
        d.status = DisputeStatus.Solved;
        // Send fee to owner (avoid blocking)
        (bool ok,) = owner.call{value: d.fee}("");
        (ok); // silence warning
        d.arbitrated.rule(_disputeID, d.ruling);
    }

    function appeal(uint256, bytes calldata) external payable override {
        revert("Appeals not supported in mock");
    }

    function appealCost(uint256, bytes calldata) external pure override returns (uint256 cost) {
        return NOT_PAYABLE_VALUE;
    }

    function appealPeriod(uint256 _disputeID) external view override returns (uint256 start, uint256 end) {
        Dispute storage d = disputes[_disputeID];
        if (d.status == DisputeStatus.Appealable) {
            start = d.appealStart;
            end = d.appealStart + appealTimeout;
        }
    }

    function disputeStatus(uint256 _disputeID) external view override returns (DisputeStatus status) {
        return disputes[_disputeID].status;
    }

    function currentRuling(uint256 _disputeID) external view override returns (uint256 ruling) {
        return disputes[_disputeID].ruling;
    }

    function disputeCount() external view returns (uint256) {
        return disputes.length;
    }
}
