@coordinator @SCOORDN01
Feature: Coordinator - Advance Funds process
  As the Union Bridge client coordinator,
  I want to orchestrate the Advance Funds process
  so that the user gets the funds advanced within the peg-out process

  Background:
    Given the Union Bridge contracts are deployed
    And BitVMX is running
    And the Union Bridge client services are running
    And the account is funded

    # Check README.md in qa-tools (Initial setup) for the setup instructions

    # Terminal window 1:

    # > (cd ../bitvmx-union-bridge-contracts && forge clean); ./run-mocking.sh

    # Watch the logs in the terminal window, search for "deployed at" to set the contract addresses appropriately in the config/qa-coordinator

    # Terminal window 2:
    # > cd qa-tools/coordinator
    # > npm install node-fetch@2 --save
    # > node anvil_proxy.js
    # NOTE: anvil proxy injects POW in blocks (it is not possible to do it via the anvil config directly) so we can
    # test reorg scenarios (block indexer weighs POW when deciding the canonical chain).
    # It also temporarily modifies common.yaml to use anvil proxy port for the provider. In this way, services
    # talk to the anvil proxy, which forwards requests to anvil and modifies the block responses to include POW.

    # Terminal window 3:
    # > ./run-client.sh --features anvil,zkp --config config/qa

    # Terminal window 4:
    # > cd qa-tools/coordinator
    # > chmod +x commands.sh
    # > source ./commands.sh 8546
    # IMPORTANT!!!
    # > fund

    # Other terminals:
    # > tail -f logs/coordinator.log
    # > tail -f logs/block-indexer.log
    # > tail -f logs/log-indexer.log

  @TCRD01001
  Scenario: happy path
    When I send the RequestAdvanceFunds event
    # > raf
    # copy pegout_id from response
    And I send the AdvanceFunds event
    # > kaf pegout_id
    And I wait for enough blocks for pow accumulation
    # > mine 5 blocks
    And I wait for enough blocks for pow confirmation
    # > mine 5 blocks
    Then the coordinator should trigger the check-fork process
    # coordinator logs should show process completion
    And the check-fork process should be successful
    # actors mocking window should display ZKP generation

  @TCRD01002
  Scenario: AdvanceFunds without RequestAdvanceFunds
    When I send the AdvanceFunds event with a fabricated pegout_id
    Then it should log an error "No blocks received yet, cannot start advance funds"

  @TCRD01003
  Scenario: wrong pegout
    When I send the RequestAdvanceFunds event
    And I send the AdvanceFunds event with a fabricated pegout_id
    Then it should log an error "AdvanceFundsData received for {pegout_id}, but no RequestAdvanceFunds was"

  @TCRD01004
  Scenario: additional RequestAdvanceFunds while in pow accumulation
    When I send the RequestAdvanceFunds event
    And I send the AdvanceFunds event
    And I wait for 1 blocks
    And I send another RequestAdvanceFunds event
    And I wait for enough blocks for pow accumulation
    And I wait for enough blocks for pow confirmation
    Then the coordinator should trigger the check-fork process
    And the check-fork process should be successful

    When I send the AdvanceFunds event for the second pegout
    And I wait for enough blocks for pow accumulation
    And I wait for enough blocks for pow confirmation
    Then the coordinator should trigger the check-fork process
    And the check-fork process should be successful

  @TCRD01005
  Scenario: additional RequestAdvanceFunds while in pow confirmation
    When I send the RequestAdvanceFunds event
    And I send the AdvanceFunds event
    And I wait for enough blocks for pow accumulation
    And I send another RequestAdvanceFunds event
    And I wait for enough blocks for pow accumulation
    And I wait for enough blocks for pow confirmation
    Then the coordinator should trigger the check-fork process
    And the check-fork process should be successful

    When I send the AdvanceFunds event for the second pegout
    And I wait for enough blocks for pow accumulation
    And I wait for enough blocks for pow confirmation
    Then the coordinator should trigger the check-fork process
    And the check-fork process should be successful

  @TCRD01006
  Scenario: duplicate AdvanceFunds event
    When I send the RequestAdvanceFunds event
    And I send the AdvanceFunds event
    And I wait for 1 blocks
    And I send the AdvanceFunds event for the same pegout_id
    Then it should log a message "Already monitoring advance funds for EventWithBlock"
    And the pow accumulation process should continue

  @TCRD01007
  Scenario: additional AdvanceFunds event - during pow accumulation
    When I send the RequestAdvanceFunds event
    And I send the AdvanceFunds event
    And I wait for 1 blocks
    And I send the RequestAdvanceFunds event
    And I send the AdvanceFunds event for the second pegout
    Then ... TBD

  @TCRD01008
  Scenario: additional AdvanceFunds event - during pow confirmation
    When I send the RequestAdvanceFunds event
    And I send the AdvanceFunds event
    And I wait for enough blocks for pow accumulation
    And I send the RequestAdvanceFunds event
    And I send the AdvanceFunds event for the second pegout
    Then ... TBD

  # Scenario currently not supported, as the coordinator does not support RemoveRequestAdvanceFunds and RemoveAdvanceFunds events
  @TCRD01009
  Scenario: RemoveRequestAdvanceFunds cancels the RequestAdvanceFunds event
    When I send the RequestAdvanceFunds event
    And I send the RemoveRequestAdvanceFunds event
    Then it should log a message "Handling RemoveRequestAdvanceFunds pegout_id"

    When I send the AdvanceFunds event
    Then it should log an error "No blocks received yet, cannot start advance funds"

  # Scenario currently not supported, as the coordinator does not support RemoveRequestAdvanceFunds and RemoveAdvanceFunds events
  @TCRD01010
  Scenario: RemoveAdvanceFunds cancels the AdvanceFunds event
    When I send the RequestAdvanceFunds event
    And I send the AdvanceFunds event
    And I wait for 1 blocks
    And I send the RemoveAdvanceFunds event
    Then it should log a message "Handling RemoveAdvanceFunds pegout_id"

    When I wait for 1 blocks
    And I send the AdvanceFunds event
    And I wait for enough blocks for pow accumulation
    And I wait for enough blocks for pow confirmation
    Then the coordinator should trigger the check-fork process
    And the check-fork process should be successful

    #TODO: for release 2, make sure this is the desired behavior - we may not want pegout_id to be valid after RemoveAdvanceFunds

  @TCRD01011
  Scenario: reorg delays pow accumulation
    When I send the RequestAdvanceFunds event
    # > raf
    # copy pegout_id from response
    And I send the AdvanceFunds event
    # > kaf pegout_id
    And I wait for 2 blocks
    # save
    # mine 2 blocks
    And a reorg happens for 2 last block
    # load
    And I wait for 3 blocks
    # mine 3 blocks
    Then it should resume pow accumulation

    When I wait for enough blocks for pow accumulation
    # mine 2 blocks
    And I wait for enough blocks for pow confirmation
    # mine 5 blocks
    Then the coordinator should trigger the check-fork process
    And the check-fork process should be successful
    # in the actors mocking window, ZKP generation should be displayed
    And the blocks accumulated in the pow should be consistent with the reorg
    # check logs and inspect the block hashes in the pow to verify

  @TCRD01012
  Scenario: reorg delays pow confirmation
    When I send the RequestAdvanceFunds event
    # > raf
    # copy pegout_id from response
    And I send the AdvanceFunds event
    # > kaf pegout_id
    And I wait for enough blocks for pow accumulation
    # mine 5 blocks
    And I wait for 2 blocks
    # save
    # mine 2 blocks
    And a reorg happens for 2 last block
    # load
    And I wait for 3 blocks
    # mine 3 blocks
    Then it should resume pow confirmation

    When I wait for enough blocks for pow confirmation
    # mine 2 blocks
    Then the coordinator should trigger the check-fork process
    And the check-fork process should be successful
    # in the actors mocking window, ZKP generation should be displayed
    And the blocks accumulated in the pow should be consistent with the reorg
    # check logs and inspect the block hashes in the pow to verify

