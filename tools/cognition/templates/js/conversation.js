/**
 * @template T
 * @typedef {Object} Edge
 * @property {string} html - HTML content of the edge
 * @property {T} data - Optional data associated with the edge
 */

/**
 * @template T
 * @class TreeNode
 */
class TreeNode {
  /**
   * @param {string} html - HTML content of the node
   * @param {T} [data] - Optional data associated with the node
   */
  constructor(html, data = undefined) {
    this.html = html;
    this.data = data;
    /** @type {Map<TreeNode<T>, Edge<T>>} */
    this.children = new Map();
  }

  /**
   * Add a child node with an edge
   * @param {TreeNode<T>} child - Child node to add
   * @param {string} edgeHtml - HTML content for the edge
   * @param {T} [edgeData] - Optional data for the edge
   * @returns {TreeNode<T>} - The added child node
   */
  addChild(child, edgeHtml, edgeData = undefined) {
    this.children.set(child, { html: edgeHtml, data: edgeData });
    return child;
  }

  /**
   * Remove a child node
   * @param {TreeNode<T>} child - Child node to remove
   * @returns {boolean} - True if the child was removed
   */
  removeChild(child) {
    return this.children.delete(child);
  }

  /**
   * Get the edge connecting to a child
   * @param {TreeNode<T>} child - Child node
   * @returns {Edge<T> | undefined} - The edge or undefined if child not found
   */
  getEdge(child) {
    return this.children.get(child);
  }

  /**
   * Get all children nodes
   * @returns {TreeNode<T>[]} - Array of child nodes
   */
  getChildren() {
    return Array.from(this.children.keys());
  }

  /**
   * Update node's HTML content
   * @param {string} html - New HTML content
   */
  updateHtml(html) {
    this.html = html;
  }

  /**
   * Update edge HTML content to a specific child
   * @param {TreeNode<T>} child - Target child node
   * @param {string} html - New HTML content for the edge
   * @returns {boolean} - True if the edge was updated
   */
  updateEdgeHtml(child, html) {
    const edge = this.children.get(child);
    if (edge) {
      edge.html = html;
      return true;
    }
    return false;
  }

  /**
   * Generate a string visualization of the tree
   * @param {string} [prefix=''] - Prefix for current line
   * @param {boolean} [isLast=true] - Is this the last child of its parent
   * @returns {string} - String representation of the tree
   */
  visualize(prefix = "", isLast = true) {
    let result =
      prefix + (prefix ? (isLast ? "└── " : "├── ") : "") + this.html + "\n";

    const children = this.getChildren();
    children.forEach((child, index) => {
      const edge = this.getEdge(child);
      const newPrefix = prefix + (isLast ? "    " : "│   ");
      const isLastChild = index === children.length - 1;

      // Add edge visualization
      result += newPrefix + (isLastChild ? "└─┤ " : "├─┤ ") + edge.html + "\n";
      result += child.visualize(newPrefix, isLastChild);
    });

    return result;
  }
}

// Unit Tests
class TestRunner {
  constructor() {
    this.tests = [];
    this.passed = 0;
    this.failed = 0;
  }

  test(name, fn) {
    this.tests.push({ name, fn });
  }

  assert(condition, message) {
    if (!condition) {
      throw new Error(message);
    }
  }

  run() {
    console.log("Running tests...\n");

    this.tests.forEach(({ name, fn }) => {
      try {
        fn();
        console.log(`✅ ${name}`);
        this.passed++;
      } catch (error) {
        console.error(`❌ ${name}`);
        console.error(`   ${error.message}`);
        this.failed++;
      }
    });

    console.log(`\nResults: ${this.passed} passed, ${this.failed} failed`);
  }
}

// Run tests
const runner = new TestRunner();

runner.test("Create node with HTML content", () => {
  const node = new TreeNode("<div>Test</div>");
  console.log("\nTree visualization:");
  console.log(node.visualize());
  runner.assert(node.html === "<div>Test</div>", "Node HTML content mismatch");
  runner.assert(node.children.size === 0, "New node should have no children");
});

runner.test("Add child with edge", () => {
  const parent = new TreeNode("<div>Parent</div>");
  const child = new TreeNode("<div>Child</div>");
  parent.addChild(child, "<line>Connection</line>");

  console.log("\nTree visualization:");
  console.log(parent.visualize());
  runner.assert(parent.children.size === 1, "Parent should have one child");
  runner.assert(
    parent.getEdge(child).html === "<line>Connection</line>",
    "Edge HTML content mismatch"
  );
});

runner.test("Remove child", () => {
  const parent = new TreeNode("<div>Parent</div>");
  const child = new TreeNode("<div>Child</div>");
  parent.addChild(child, "<line>Connection</line>");

  console.log("\nBefore removal:");
  console.log(parent.visualize());

  const removed = parent.removeChild(child);

  console.log("\nAfter removal:");
  console.log(parent.visualize());
  runner.assert(removed === true, "Child removal should return true");
  runner.assert(
    parent.children.size === 0,
    "Parent should have no children after removal"
  );
});

runner.test("Update node and edge HTML", () => {
  const parent = new TreeNode("<div>Parent</div>");
  const child = new TreeNode("<div>Child</div>");
  parent.addChild(child, "<line>Connection</line>");

  console.log("\nBefore updates:");
  console.log(parent.visualize());

  parent.updateHtml("<div>Updated Parent</div>");
  parent.updateEdgeHtml(child, "<line>Updated Connection</line>");

  console.log("\nAfter updates:");
  console.log(parent.visualize());
  runner.assert(
    parent.html === "<div>Updated Parent</div>",
    "Node HTML update failed"
  );
  runner.assert(
    parent.getEdge(child).html === "<line>Updated Connection</line>",
    "Edge HTML update failed"
  );
});

runner.test("Generic type support", () => {
  const parent = new TreeNode("<div>Parent</div>", { id: 1 });
  const child = new TreeNode("<div>Child</div>", { id: 2 });
  parent.addChild(child, "<line>Connection</line>", { weight: 10 });

  console.log("\nTree visualization (with data):");
  console.log(parent.visualize());
  runner.assert(parent.data.id === 1, "Node data mismatch");
  runner.assert(parent.getEdge(child).data.weight === 10, "Edge data mismatch");
});

runner.test("Complex nested tree structure", () => {
  // Create a complex family tree with multiple generations and relationships
  const root = new TreeNode("<div>Grandparent</div>", { generation: 1 });

  // First generation children
  const parent1 = new TreeNode("<div>Parent 1</div>", { generation: 2 });
  const parent2 = new TreeNode("<div>Parent 2</div>", { generation: 2 });
  const parent3 = new TreeNode("<div>Parent 3</div>", { generation: 2 });

  root.addChild(parent1, "<line>First child</line>", { relation: "eldest" });
  root.addChild(parent2, "<line>Second child</line>", { relation: "middle" });
  root.addChild(parent3, "<line>Third child</line>", { relation: "youngest" });

  // Second generation - Parent 1's children
  const child1A = new TreeNode("<div>Child 1A</div>", { generation: 3 });
  const child1B = new TreeNode("<div>Child 1B</div>", { generation: 3 });
  parent1.addChild(child1A, "<line>First grandchild</line>", {
    branch: "left",
  });
  parent1.addChild(child1B, "<line>Second grandchild</line>", {
    branch: "left",
  });

  // Second generation - Parent 2's child
  const child2A = new TreeNode("<div>Child 2A</div>", { generation: 3 });
  parent2.addChild(child2A, "<line>Third grandchild</line>", {
    branch: "middle",
  });

  // Third generation - Child 1A's children
  const greatChild1 = new TreeNode("<div>Great-Grandchild 1</div>", {
    generation: 4,
  });
  const greatChild2 = new TreeNode("<div>Great-Grandchild 2</div>", {
    generation: 4,
  });
  child1A.addChild(greatChild1, "<line>First great-grandchild</line>", {
    branch: "leftmost",
  });
  child1A.addChild(greatChild2, "<line>Second great-grandchild</line>", {
    branch: "leftmost",
  });

  console.log("\nComplex tree visualization:");
  console.log(root.visualize());

  // Assertions to verify the structure
  runner.assert(root.getChildren().length === 3, "Root should have 3 children");
  runner.assert(
    parent1.getChildren().length === 2,
    "Parent 1 should have 2 children"
  );
  runner.assert(
    parent2.getChildren().length === 1,
    "Parent 2 should have 1 child"
  );
  runner.assert(
    parent3.getChildren().length === 0,
    "Parent 3 should have no children"
  );
  runner.assert(
    child1A.getChildren().length === 2,
    "Child 1A should have 2 children"
  );

  // Verify data and relationships
  runner.assert(root.data.generation === 1, "Root should be generation 1");
  runner.assert(
    parent1.data.generation === 2,
    "Parent 1 should be generation 2"
  );
  runner.assert(
    child1A.data.generation === 3,
    "Child 1A should be generation 3"
  );
  runner.assert(
    greatChild1.data.generation === 4,
    "Great-Grandchild 1 should be generation 4"
  );

  // Verify edge data
  runner.assert(
    root.getEdge(parent1).data.relation === "eldest",
    "Parent 1 should be eldest child"
  );
  runner.assert(
    child1A.getEdge(greatChild1).data.branch === "leftmost",
    "Great-Grandchild 1 should be on leftmost branch"
  );
});

// Run all tests
runner.run();

// Export for use in other modules
if (typeof module !== "undefined" && module.exports) {
  module.exports = { TreeNode };
}
