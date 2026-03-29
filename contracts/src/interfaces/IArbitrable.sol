// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./IArbitrator.sol";

/// @title ERC-792 Arbitrable interface
interface IArbitrable {
    event Ruling(IArbitrator indexed _arbitrator, uint256 indexed _disputeID, uint256 _ruling);

    function rule(uint256 _disputeID, uint256 _ruling) external;
}
