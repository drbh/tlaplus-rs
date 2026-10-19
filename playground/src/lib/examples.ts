// Examples are bundled from the repository, not maintained as separate copies.
import type {Language} from './types';
const sources = import.meta.glob('../../../examples/*/*.{tsl,tla}',{query:'?raw',import:'default',eager:true}) as Record<string,string>;
const picks = [
 ['Die Hard jugs','16-puzzles/DieHard','Find four gallons. NotSolved intentionally fails.'],
 ['Typed jugs','16-puzzles/DieHardTyped','Two distinct jug types; the same six-move solution.'],
 ['River crossing','16-puzzles/RiverCrossing','Find a safe seven-crossing route.'],
 ['Counter','01-counter/Counter','A complete four-state cycle.'],
 ['Race condition','08-race/Race','Two threads lose an update.'],
 ['Deadlock','10-deadlock/Deadlock','Opposite lock orders block both threads.'],
 ['Unfair retry','17-modern-dsl/UnfairRetry','A liveness counterexample repeats forever.'],
 ['Reliable delivery','18-reliable-delivery/ReliableDelivery','A protocol with retries, fairness, and a hundred states.'],
 ['Knights and knaves','19-puzzle-gallery/KnightsAndKnaves','Find who tells the truth. The solution is an initial assignment, not an action sequence.','knights'],
 ['Feature selection','19-puzzle-gallery/FeatureSelection','Choose three compatible features within budget. There are three valid packages.','features'],
 ['Mini Einstein','19-puzzle-gallery/MiniEinstein','Match three houses, jobs, and pets from the clues. One consistent assignment.','houses'],
 ['Cryptarithm','19-puzzle-gallery/Cryptarithm','Solve I + BB = ILL with distinct digits and no leading zeroes.','digits'],
 ['4×4 Sudoku','19-puzzle-gallery/MiniSudoku','Fill the seven empty cells. Each step places one digit; fixed clues stay unchanged.','sudoku'],
 ['Bridge and torch','19-puzzle-gallery/BridgeAndTorch','Find a route within 17 minutes. Change Budget to 16 to check impossibility.','bridge'],
 ['Warehouse robot','19-puzzle-gallery/WarehouseRobot','Push the crate to (3,3). Coordinates start at the top left; pulling is forbidden.','warehouse'],
 ['Staff rota','19-puzzle-gallery/StaffRota','Schedule six shifts with availability, rest, and workload constraints.','rota'],
 ['Mini factory','19-puzzle-gallery/MiniFactory','Schedule six operations by time 5. Change Deadline to 4 to check infeasibility.','factory'],
 ['Paxos · agreement','20-paxos/Paxos','Three acceptors, two ballots. Checks safety across 1,696 states; allow several seconds.','paxos','2000'],
 ['Paxos · broken proposer','20-paxos/BrokenPaxos','Ignore the inherited value and find two different chosen values in ten steps.','paxos','2000'],
];
export const examples = picks.map(([title,path,note,puzzle,budget])=>({title,path,note,puzzle,maxStates:Number(budget??500),
  tsl:sources[`../../../examples/${path}.tsl`],tla:sources[`../../../examples/${path}.tla`]}));
export function sourceFor(index:number,language:Language) { return examples[index][language] ?? ''; }
