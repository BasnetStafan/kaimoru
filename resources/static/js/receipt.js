// The receipt page's only behaviour: the print button.
// The page is standalone (no site chrome) so the printed
// sheet is the receipt alone; print CSS hides the actions.
(function () {
    'use strict';
    var button = document.querySelector('[data-print]');
    if (button) {
        button.addEventListener('click', function () {
            window.print();
        });
    }
})();
